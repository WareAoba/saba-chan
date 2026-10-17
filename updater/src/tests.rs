//! UpdateManager 단위 테스트 — 압축 해제 안전성, 매니페스트 검증, 적용·롤백

use crate::{Component, ComponentVersion, UpdateConfig, UpdateManager};
use std::path::{Path, PathBuf};

fn test_config(relay_url: &str) -> UpdateConfig {
    UpdateConfig {
        enabled: true,
        check_interval_hours: 1,
        auto_download: false,
        auto_apply: false,
        relay_base_url: relay_url.to_string(),
        channel: "stable".to_string(),
        platform: "windows-x64".to_string(),
        client_id: "test-client-id".to_string(),
        install_root: Some("./test_install".to_string()),
    }
}

/// 실제 %APPDATA% 를 건드리지 않도록 모든 경로를 임시 디렉터리에 둔 매니저
fn isolated_manager(root: &Path) -> UpdateManager {
    UpdateManager {
        config: test_config("http://127.0.0.1:1"),
        status: crate::UpdateStatus {
            last_check: None,
            next_check: None,
            components: Vec::new(),
            checking: false,
            error: None,
        },
        modules_dir: root.join("modules"),
        extensions_dir: root.join("extensions"),
        staging_dir: root.join("staging"),
        install_root: root.join("install"),
        cached_manifest: None,
        public_keys: crate::signature::PublicKeyRegistry::bundled(),
        resolved_components: Default::default(),
        cached_release_notes: None,
        cached_published_at: None,
        download_progress: Default::default(),
    }
}

fn write_test_zip(path: &Path, entries: &[(&str, &[u8])]) {
    use std::io::Write;
    let mut archive = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let options = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);
    for (name, contents) in entries {
        archive.start_file(*name, options).unwrap();
        archive.write_all(contents).unwrap();
    }
    archive.finish().unwrap();
}

/// 지정한 페이로드 바이트를 뒤집어 CRC 오류를 만든다 (해당 엔트리 추출 중 실패).
fn corrupt_payload(archive: &Path, payload: &[u8]) {
    let mut bytes = std::fs::read(archive).unwrap();
    let start = bytes.windows(payload.len()).position(|part| part == payload).unwrap();
    bytes[start] ^= 1;
    std::fs::write(archive, bytes).unwrap();
}

fn sha256_of(path: &Path) -> String {
    crate::integrity::hash_file(path).unwrap().to_hex_lower()
}

/// 다운로드 완료 상태의 대기 컴포넌트
fn pending_component(component: Component, staged: &Path, sha256: Option<String>) -> ComponentVersion {
    ComponentVersion {
        component,
        current_version: "1.0.0".to_string(),
        latest_version: Some("2.0.0".to_string()),
        update_available: true,
        download_url: None,
        asset_name: None,
        release_notes: None,
        published_at: None,
        downloaded: true,
        downloaded_path: Some(staged.to_string_lossy().to_string()),
        installed: true,
        sha256,
    }
}

// ═══════════════════════════════════════════════════════
// 압축 해제 안전성
// ═══════════════════════════════════════════════════════

#[test]
fn archive_rejects_zip_escape_before_writing_any_entries() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("payload.zip");
    let target = root.path().join("target");
    write_test_zip(&archive, &[("valid.txt", b"valid"), ("../escaped.txt", b"escape")]);

    assert!(crate::extract::extract_to_directory(&archive, &target).is_err());
    assert!(!root.path().join("escaped.txt").exists());
    assert!(!target.join("valid.txt").exists());
}

#[test]
fn archive_rejects_windows_zip_escape() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("payload.zip");
    write_test_zip(&archive, &[("..\\escaped.txt", b"escape")]);

    assert!(crate::extract::extract_to_directory(&archive, &root.path().join("target")).is_err());
    assert!(!root.path().join("escaped.txt").exists());
}

#[test]
fn archive_rejects_tar_link_entries() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("keep.txt"), b"old").unwrap();
    let archive = root.path().join("payload.tar.gz");
    let encoder = flate2::write::GzEncoder::new(std::fs::File::create(&archive).unwrap(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let mut file_header = tar::Header::new_gnu();
    file_header.set_size(3);
    file_header.set_mode(0o644);
    file_header.set_cksum();
    builder.append_data(&mut file_header, "keep.txt", &b"new"[..]).unwrap();
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_mode(0o777);
    header.set_size(0);
    builder.append_link(&mut header, "escape", root.path().join("outside")).unwrap();
    builder.into_inner().unwrap().finish().unwrap();

    assert!(crate::extract::extract_to_directory(&archive, &target).is_err());
    assert_eq!(std::fs::read(target.join("keep.txt")).unwrap(), b"old");
}

#[test]
fn archive_paths_reject_absolute_and_drive_names() {
    use crate::extract::archive_entry_path;
    for name in ["/absolute", "//server/share/file", "C:/absolute", "C:drive-relative", "file:stream", "./../escape"] {
        assert!(archive_entry_path(Path::new("target"), name).is_err(), "accepted {name}");
    }
    assert!(archive_entry_path(Path::new("target"), "./nested/file..txt").is_ok());
    assert!(archive_entry_path(Path::new("target"), "./").is_ok());
}

#[test]
fn archive_extracts_valid_nested_zip() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("payload.zip");
    let target = root.path().join("target");
    write_test_zip(&archive, &[("nested/file..txt", b"valid")]);
    crate::extract::extract_to_directory(&archive, &target).unwrap();
    assert_eq!(std::fs::read(target.join("nested/file..txt")).unwrap(), b"valid");
}

// ═══════════════════════════════════════════════════════
// 디렉터리 업데이트 (모듈·익스텐션) 롤백
// ═══════════════════════════════════════════════════════

#[test]
fn directory_update_failure_restores_previous_files() {
    let root = tempfile::tempdir().unwrap();
    let manager = isolated_manager(root.path());
    let target = manager.modules_dir.join("test");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("old.py"), b"previous release").unwrap();
    let archive = root.path().join("payload.zip");
    write_test_zip(&archive, &[("new.py", b"unique-new-release-payload")]);
    corrupt_payload(&archive, b"unique-new-release-payload");

    assert!(manager.apply_module_update("test", &archive).is_err());
    assert_eq!(std::fs::read(target.join("old.py")).unwrap(), b"previous release");
    assert!(!target.join("new.py").exists());
    assert!(archive.exists());
}

#[test]
fn directory_update_rejects_non_archive_without_touching_target() {
    let root = tempfile::tempdir().unwrap();
    let manager = isolated_manager(root.path());
    let target = manager.modules_dir.join("test");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("old.py"), b"previous release").unwrap();
    let staged = root.path().join("payload.bin");
    std::fs::write(&staged, b"not an archive").unwrap();

    let err = manager.apply_module_update("test", &staged).unwrap_err();
    assert!(err.to_string().contains("Unsupported package format"), "{err:#}");
    assert!(target.is_dir());
    assert_eq!(std::fs::read(target.join("old.py")).unwrap(), b"previous release");
}

#[test]
fn directory_update_success_removes_backup_and_staged_file() {
    let root = tempfile::tempdir().unwrap();
    let manager = isolated_manager(root.path());
    let target = manager.modules_dir.join("test");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("old.py"), b"previous release").unwrap();
    let archive = root.path().join("payload.zip");
    write_test_zip(&archive, &[("new.py", b"new")]);

    manager.apply_module_update("test", &archive).unwrap();
    assert!(!target.join("old.py").exists());
    assert_eq!(std::fs::read(target.join("new.py")).unwrap(), b"new");
    assert!(!manager.staging_dir.join("test_backup").exists(), "backup must be cleaned up");
    assert!(!archive.exists());
}

// ═══════════════════════════════════════════════════════
// 바이너리 업데이트 롤백
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn binary_update_failure_restores_previous_files() {
    let root = tempfile::tempdir().unwrap();
    let manager = isolated_manager(root.path());
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let exe = manager.install_root.join("saba-test.exe");
    std::fs::write(&exe, b"V1").unwrap();
    std::fs::write(manager.install_root.join("data.txt"), b"old data").unwrap();
    let archive = root.path().join("core.zip");
    write_test_zip(&archive, &[
        ("saba-test.exe", b"V2"),
        ("data.txt", b"unique-corrupted-data-payload"),
    ]);
    corrupt_payload(&archive, b"unique-corrupted-data-payload");

    assert!(manager.apply_binary_update("test-bin", &archive).await.is_err());
    assert_eq!(std::fs::read(&exe).unwrap(), b"V1");
    assert_eq!(std::fs::read(manager.install_root.join("data.txt")).unwrap(), b"old data");
    assert!(!manager.install_root.join("saba-test.exe.old").exists());
    assert!(!manager.install_root.join("data.txt.old").exists());
}

#[tokio::test]
async fn binary_update_failure_removes_newly_created_files() {
    let root = tempfile::tempdir().unwrap();
    let manager = isolated_manager(root.path());
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let archive = root.path().join("core.zip");
    write_test_zip(&archive, &[
        ("new-file.dll", b"new"),
        ("broken.dll", b"unique-broken-payload"),
    ]);
    corrupt_payload(&archive, b"unique-broken-payload");

    assert!(manager.apply_binary_update("test-bin", &archive).await.is_err());
    assert!(!manager.install_root.join("new-file.dll").exists());
    assert!(!manager.install_root.join("broken.dll").exists());
}

#[tokio::test]
async fn binary_update_success_replaces_files_and_cleans_backups() {
    let root = tempfile::tempdir().unwrap();
    let manager = isolated_manager(root.path());
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let exe = manager.install_root.join("saba-test.exe");
    std::fs::write(&exe, b"V1").unwrap();
    let archive = root.path().join("core.zip");
    write_test_zip(&archive, &[("saba-test.exe", b"V2")]);

    manager.apply_binary_update("test-bin", &archive).await.unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), b"V2");
    assert!(!manager.install_root.join("saba-test.exe.old").exists());
    assert!(!archive.exists());
}

#[tokio::test]
async fn binary_update_rejects_unsupported_package() {
    let root = tempfile::tempdir().unwrap();
    let manager = isolated_manager(root.path());
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let staged = root.path().join("core.bin");
    std::fs::write(&staged, b"raw").unwrap();

    assert!(manager.apply_binary_update("test-bin", &staged).await.is_err());
    assert!(staged.exists(), "staged file must be kept on failure");
}

// ═══════════════════════════════════════════════════════
// SHA-256 필수 (다운로드·적용)
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn download_requires_manifest_sha256() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = isolated_manager(root.path());
    let component = Component::Locales;
    let mut status = pending_component(component.clone(), &root.path().join("unused.zip"), None);
    status.downloaded = false;
    status.downloaded_path = None;
    manager.status.components.push(status);
    manager.resolved_components.insert("locales".to_string(), crate::ResolvedComponent {
        latest_version: "2.0.0".to_string(),
        source_release_tag: "2.0.0".to_string(),
        download_url: "http://127.0.0.1:1/locales.zip".to_string(),
        asset_name: "locales.zip".to_string(),
        install_dir: None,
        sha256: None,
        requires: None,
    });

    let err = manager.download_component(&component).await.unwrap_err();
    assert!(err.to_string().contains("sha256"), "{err:#}");
    assert!(!manager.staging_dir.join("locales.zip").exists());
}

#[tokio::test]
async fn apply_rejects_staged_file_that_changed_after_download() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = isolated_manager(root.path());
    std::fs::create_dir_all(&manager.staging_dir).unwrap();
    let staged = manager.staging_dir.join("module-test.zip");
    write_test_zip(&staged, &[("module.toml", b"name = \"test\"")]);
    let downloaded_hash = sha256_of(&staged);
    write_test_zip(&staged, &[("module.toml", b"name = \"evil\"")]);

    let component = Component::Module("test".to_string());
    manager.status.components.push(pending_component(component.clone(), &staged, Some(downloaded_hash)));

    let err = manager.apply_single_component(&component).await.unwrap_err();
    assert!(format!("{err:#}").contains("verification failed"), "{err:#}");
    assert!(!manager.modules_dir.join("test").exists());
}

#[tokio::test]
async fn apply_rejects_pending_component_without_sha256() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = isolated_manager(root.path());
    std::fs::create_dir_all(&manager.staging_dir).unwrap();
    let staged = manager.staging_dir.join("module-test.zip");
    write_test_zip(&staged, &[("module.toml", b"name = \"test\"")]);
    let component = Component::Module("test".to_string());
    manager.status.components.push(pending_component(component.clone(), &staged, None));

    assert!(manager.apply_single_component(&component).await.is_err());
    assert!(!manager.modules_dir.join("test").exists());
}

// ═══════════════════════════════════════════════════════
// 일괄 적용 — 실패 항목은 pending 으로 남는다
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn apply_pending_keeps_failed_components_pending() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = isolated_manager(root.path());
    std::fs::create_dir_all(&manager.staging_dir).unwrap();

    let good = manager.staging_dir.join("module-good.zip");
    write_test_zip(&good, &[("module.toml", b"name = \"good\"")]);
    let bad = manager.staging_dir.join("module-bad.zip");
    write_test_zip(&bad, &[("module.toml", b"name = \"bad\"")]);

    manager.status.components = vec![
        pending_component(Component::Module("good".into()), &good, Some(sha256_of(&good))),
        pending_component(Component::Module("bad".into()), &bad, Some("00".repeat(32))),
    ];
    manager.save_pending_manifest().unwrap();
    manager.save_updater_apply_targets(&["module-good".into(), "module-bad".into()]).unwrap();

    let mut reloaded = isolated_manager(root.path());
    assert_eq!(reloaded.load_pending_manifest().unwrap(), 2);
    let keys = reloaded.load_updater_apply_targets().unwrap();
    let mut progress_calls = Vec::new();
    let outcome = reloaded.apply_pending(&keys, |i, key| progress_calls.push((i, key.to_string()))).await;

    assert_eq!(outcome.applied, vec!["module-good".to_string()]);
    assert_eq!(outcome.failed_keys(), vec!["module-bad".to_string()]);
    assert_eq!(progress_calls.len(), 2);
    assert!(manager.modules_dir.join("good").join("module.toml").exists());

    let mut after = isolated_manager(root.path());
    assert_eq!(after.load_pending_manifest().unwrap(), 1, "failed component must stay pending");
    assert_eq!(after.get_pending_components()[0].component, Component::Module("bad".into()));
}

#[tokio::test]
async fn apply_pending_clears_state_files_when_all_succeed() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = isolated_manager(root.path());
    std::fs::create_dir_all(&manager.staging_dir).unwrap();
    let good = manager.staging_dir.join("module-good.zip");
    write_test_zip(&good, &[("module.toml", b"name = \"good\"")]);
    manager.status.components = vec![
        pending_component(Component::Module("good".into()), &good, Some(sha256_of(&good))),
    ];
    manager.save_pending_manifest().unwrap();

    let outcome = manager.apply_pending(&["module-good".into()], |_, _| {}).await;
    assert!(outcome.failed.is_empty());
    assert!(!manager.staging_dir.join("pending.json").exists());
}

#[test]
fn save_pending_manifest_removes_stale_file_when_nothing_pending() {
    let root = tempfile::tempdir().unwrap();
    let manager = isolated_manager(root.path());
    std::fs::create_dir_all(&manager.staging_dir).unwrap();
    let pending = manager.staging_dir.join("pending.json");
    std::fs::write(&pending, "[]").unwrap();

    manager.save_pending_manifest().unwrap();
    assert!(!pending.exists());
}

#[test]
fn pending_manifest_without_sha256_field_still_loads() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = isolated_manager(root.path());
    std::fs::create_dir_all(&manager.staging_dir).unwrap();
    let staged = manager.staging_dir.join("cli.zip");
    std::fs::write(&staged, b"x").unwrap();
    let legacy = serde_json::json!([{
        "component": "cli", "current_version": "1.0.0", "latest_version": "2.0.0",
        "update_available": true, "download_url": null, "asset_name": null,
        "release_notes": null, "published_at": null, "downloaded": true,
        "downloaded_path": staged, "installed": true
    }]);
    std::fs::write(manager.staging_dir.join("pending.json"), legacy.to_string()).unwrap();

    assert_eq!(manager.load_pending_manifest().unwrap(), 1);
    assert!(manager.get_pending_components()[0].sha256.is_none());
}

// ═══════════════════════════════════════════════════════
// 매니페스트 경로 검증
// ═══════════════════════════════════════════════════════

#[test]
fn metadata_rejects_paths_outside_component_directories() {
    let root = tempfile::tempdir().unwrap();
    let manager = isolated_manager(root.path());
    for component in [
        serde_json::json!({"cli": {"version": "1.0.0", "asset": "../outside.zip"}}),
        serde_json::json!({"cli": {"version": "1.0.0", "install_dir": "../outside"}}),
        serde_json::json!({"module-../outside": {"version": "1.0.0"}}),
        serde_json::json!({"ext-..\\outside": {"version": "1.0.0"}}),
        serde_json::json!({"module-": {"version": "1.0.0"}}),
    ] {
        let manifest: crate::ReleaseManifest = serde_json::from_value(serde_json::json!({
            "version": "1.0.0", "components": component
        })).unwrap();
        assert!(manager.validate_manifest_paths(&manifest).is_err(), "accepted {:?}", manifest.components);
    }
}

#[test]
fn metadata_accepts_root_and_nested_install_directories() {
    let root = tempfile::tempdir().unwrap();
    let manager = isolated_manager(root.path());
    let manifest: crate::ReleaseManifest = serde_json::from_value(serde_json::json!({
        "version": "1.0.0", "components": {
            "cli": {"version": "1.0.0", "install_dir": ".", "asset": "cli.zip"},
            "ext-test": {"version": "1.0.0", "install_dir": "extensions/test-name"}
        }
    })).unwrap();
    assert!(manager.validate_manifest_paths(&manifest).is_ok());
}

#[tokio::test]
async fn install_rejects_hash_mismatch() {
    use base64::Engine;
    use ed25519_dalek::Signer;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let root = tempfile::tempdir().unwrap();
    let mut manager = isolated_manager(root.path());
    let archive = root.path().join("locales.zip");
    write_test_zip(&archive, &[("en/gui.json", b"{}")]);
    let payload = std::fs::read(&archive).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let sk = ed25519_dalek::SigningKey::from_bytes(&[0x5a; 32]);
    manager.public_keys = crate::signature::PublicKeyRegistry::from_entries(vec![("test".into(), sk.verifying_key())]);
    manager.config.relay_base_url = base_url.clone();
    let manifest = serde_json::json!({"version": "1.0.0", "components": {
        "locales": {"version": "1.0.0", "asset": "locales.zip", "url": format!("{base_url}/asset"), "sha256": "00".repeat(32)}
    }});
    let signed = serde_json::json!({
        "signature": base64::engine::general_purpose::STANDARD.encode(sk.sign(&crate::signature::canonical_json_bytes(&manifest).unwrap()).to_bytes()),
        "manifest": manifest, "keyId": "test"
    });
    let check = serde_json::json!({"upToDate": false, "latest": {
        "version": "1.0.0", "channel": "stable", "releasedAt": "2026-01-01T00:00:00Z"
    }});
    let server = tokio::spawn(async move {
        for body in [serde_json::to_vec(&check).unwrap(), serde_json::to_vec(&signed).unwrap(), payload] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let mut received = 0;
            while !request[..received].windows(4).any(|part| part == b"\r\n\r\n") {
                let size = stream.read(&mut request[received..]).await.unwrap();
                assert!(size > 0 && received + size < request.len());
                received += size;
            }
            let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(&body).await.unwrap();
        }
    });

    let result = manager.install_component(&Component::Locales).await;
    server.await.unwrap();
    assert!(result.is_err());
    assert!(!manager.install_root.join("locales/en/gui.json").exists());
}

#[test]
fn resolved_components_skip_entries_without_url() {
    let manifest: crate::ReleaseManifest = serde_json::from_value(serde_json::json!({
        "version": "1.0.0", "components": {
            "cli": {"version": "1.0.0", "url": "https://cdn/cli.zip", "sha256": "ab"},
            "gui": {"version": "1.0.0"}
        }
    })).unwrap();
    let resolved = UpdateManager::build_resolved_components(&manifest);
    assert!(resolved.contains_key("cli"));
    assert!(!resolved.contains_key("gui"));
    assert_eq!(resolved["cli"].asset_name, "cli.zip");
}

// ═══════════════════════════════════════════════════════
// 컴포넌트 키 / 매니페스트 스키마
// ═══════════════════════════════════════════════════════

#[test]
fn component_manifest_key_roundtrip() {
    assert_eq!(Component::CoreDaemon.manifest_key(), "saba-core");
    assert_eq!(Component::Cli.manifest_key(), "cli");
    assert_eq!(Component::Gui.manifest_key(), "gui");
    assert_eq!(Component::Module("minecraft".to_string()).manifest_key(), "module-minecraft");
    assert_eq!(Component::Extension("docker".to_string()).manifest_key(), "ext-docker");

    assert_eq!(Component::from_manifest_key("saba-core"), Component::CoreDaemon);
    assert_eq!(Component::from_manifest_key("module-palworld"), Component::Module("palworld".to_string()));
    assert_eq!(Component::from_manifest_key("ext-music"), Component::Extension("music".to_string()));
    assert_eq!(Component::try_from_manifest_key("installer"), None);

    assert_eq!(Component::CoreDaemon.display_name(), "Saba-Core");
    assert_eq!(Component::Extension("docker".to_string()).display_name(), "Extension: docker");
}

#[test]
fn component_info_requires_field() {
    let json = r#"{"version": "0.3.0", "asset": "gui-windows-x64.zip", "requires": {"saba-core": ">=0.3.0"}}"#;
    let info: crate::ComponentInfo = serde_json::from_str(json).unwrap();
    assert_eq!(info.version, "0.3.0");
    assert_eq!(info.requires.unwrap().get("saba-core").unwrap(), ">=0.3.0");

    let info: crate::ComponentInfo = serde_json::from_str(r#"{"version": "0.1.0"}"#).unwrap();
    assert!(info.requires.is_none());
}

#[test]
fn newer_version_detection() {
    assert!(UpdateManager::is_newer_version("1.0.1", "1.0.0"));
    assert!(UpdateManager::is_newer_version("1.0.0", ""));
    assert!(!UpdateManager::is_newer_version("1.0.0", "1.0.0"));
    assert!(!UpdateManager::is_newer_version("garbage", "1.0.0"));
}

#[allow(dead_code)]
fn _assert_paths_are_pathbuf(_: PathBuf) {}
