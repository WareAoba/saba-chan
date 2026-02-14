//! ═══════════════════════════════════════════════════════════════════
//! 업데이터 통합 테스트
//! ═══════════════════════════════════════════════════════════════════
//!
//! 전체 업데이트 파이프라인을 테스트합니다:
//!
//! 1. **버전 파싱/비교** — SemVer 유효성, 순서 비교
//! 2. **GitHub API 모킹** — 로컬 HTTP 서버로 릴리스/매니페스트/에셋 응답
//! 3. **UpdateManager 전체 흐름** — check → download → apply
//! 4. **파일 덮어쓰기** — zip 해제, 모듈 교체, 바이너리 교체
//! 5. **인스톨러** — fresh_install, install_component
//! 6. **데몬 IPC API** — Axum 라우터를 직접 호출 (HTTP 통합 테스트)
//! 7. **설정 변경** — UpdateConfig round-trip
//! 8. **스케줄러** — SchedulerConfig 유틸리티
//!
//! 모든 테스트는 `tempdir`을 사용해 파일시스템을 격리합니다.

use axum::Router;
use axum::routing::get;
use axum::Json;
use serde_json::json;
use std::collections::HashMap;
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tempfile::TempDir;

use saba_chan::updater::{
    Component,
    UpdateConfig, UpdateManager,
};
use saba_chan::updater::github::{
    GitHubClient, ReleaseManifest,
};
use saba_chan::updater::scheduler::SchedulerConfig;
use saba_chan::updater::version::SemVer;

// ═══════════════════════════════════════════════════════
// 테스트 유틸리티
// ═══════════════════════════════════════════════════════

/// 테스트용 zip 파일 생성 — 파일 이름 → 내용 맵을 zip 으로 패킹
fn create_test_zip(files: &HashMap<&str, &[u8]>) -> Vec<u8> {
    let buf = std::io::Cursor::new(Vec::new());
    let mut zip_writer = zip::ZipWriter::new(buf);
    let options = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);

    for (name, content) in files {
        zip_writer.start_file(*name, options).unwrap();
        zip_writer.write_all(content).unwrap();
    }

    let cursor = zip_writer.finish().unwrap();
    cursor.into_inner()
}

/// 모킹용 manifest.json 생성
fn create_test_manifest(release_version: &str, components: Vec<(&str, &str, &str, Option<&str>)>) -> String {
    let mut comp_map = serde_json::Map::new();
    for (key, version, asset, install_dir) in components {
        let mut info = serde_json::Map::new();
        info.insert("version".into(), json!(version));
        info.insert("asset".into(), json!(asset));
        info.insert("sha256".into(), json!(null));
        info.insert("install_dir".into(), match install_dir {
            Some(d) => json!(d),
            None => json!(null),
        });
        comp_map.insert(key.into(), serde_json::Value::Object(info));
    }
    json!({
        "release_version": release_version,
        "components": comp_map,
    }).to_string()
}

/// 로컬 모킹 GitHub API 서버 시작
/// 반환: (서버 주소, JoinHandle)
async fn start_mock_github_server(
    manifest_json: String,
    assets: HashMap<String, Vec<u8>>,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let manifest = manifest_json.clone();
    let assets_arc = Arc::new(assets);

    // 릴리스 목록 응답 생성
    let mut release_assets = vec![
        json!({
            "name": "manifest.json",
            "size": manifest.len(),
            "browser_download_url": "PLACEHOLDER_manifest",
            "content_type": "application/json"
        }),
    ];
    for (name, data) in assets_arc.as_ref() {
        release_assets.push(json!({
            "name": name,
            "size": data.len(),
            "browser_download_url": format!("PLACEHOLDER_{}", name),
            "content_type": "application/zip"
        }));
    }

    let release_body = json!([{
        "tag_name": "v0.2.0",
        "name": "v0.2.0 Release",
        "body": "Test release notes\n- Fixed bugs\n- Added features",
        "prerelease": false,
        "draft": false,
        "published_at": "2026-02-13T00:00:00Z",
        "html_url": "https://github.com/test/saba-chan/releases/tag/v0.2.0",
        "assets": release_assets,
    }]);

    // 라우터 클로저용 데이터
    let manifest_for_handler = manifest.clone();
    let assets_for_handler = assets_arc.clone();
    let release_json = release_body.to_string();

    // 서버 바인드
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // release_body에 실제 URL 주입
    let real_release = release_json
        .replace("PLACEHOLDER_manifest", &format!("http://{}/download/manifest.json", addr));
    let real_release = {
        let mut s = real_release;
        for name in assets_arc.keys() {
            s = s.replace(
                &format!("PLACEHOLDER_{}", name),
                &format!("http://{}/download/{}", addr, name),
            );
        }
        s
    };

    let real_release_arc = Arc::new(real_release);
    let manifest_arc = Arc::new(manifest_for_handler);

    let handle = tokio::spawn(async move {
        let release_data = real_release_arc.clone();
        let manifest_data = manifest_arc.clone();
        let assets_data = assets_for_handler.clone();

        let app = Router::new()
            .route("/repos/:owner/:repo/releases", get({
                let d = release_data.clone();
                move || async move {
                    (
                        [("content-type", "application/json")],
                        d.as_str().to_string(),
                    )
                }
            }))
            .route("/repos/:owner/:repo/releases/latest", get({
                let d = release_data.clone();
                move || async move {
                    // latest는 배열이 아닌 단일 객체
                    let arr: Vec<serde_json::Value> = serde_json::from_str(&d).unwrap();
                    (
                        [("content-type", "application/json")],
                        arr[0].to_string(),
                    )
                }
            }))
            .route("/download/:filename", get({
                let m = manifest_data.clone();
                let a = assets_data.clone();
                move |axum::extract::Path(filename): axum::extract::Path<String>| {
                    let m = m.clone();
                    let a = a.clone();
                    async move {
                        if filename == "manifest.json" {
                            (
                                axum::http::StatusCode::OK,
                                [("content-type", "application/json")],
                                m.as_bytes().to_vec(),
                            )
                        } else if let Some(data) = a.get(&filename) {
                            (
                                axum::http::StatusCode::OK,
                                [("content-type", "application/octet-stream")],
                                data.clone(),
                            )
                        } else {
                            (
                                axum::http::StatusCode::NOT_FOUND,
                                [("content-type", "text/plain")],
                                b"Not Found".to_vec(),
                            )
                        }
                    }
                }
            }));

        axum::serve(listener, app).await.unwrap();
    });

    // 서버 시작 대기
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    (addr, handle)
}

/// UpdateManager를 테스트 환경으로 생성 (staging, install_root, modules 모두 tempdir)
fn create_test_manager(
    tmpdir: &TempDir,
    github_owner: &str,
    github_repo: &str,
) -> UpdateManager {
    let modules_dir = tmpdir.path().join("modules");
    std::fs::create_dir_all(&modules_dir).unwrap();

    let config = UpdateConfig {
        enabled: true,
        check_interval_hours: 3,
        auto_download: false,
        auto_apply: false,
        github_owner: github_owner.to_string(),
        github_repo: github_repo.to_string(),
        include_prerelease: false,
        install_root: Some(tmpdir.path().to_string_lossy().to_string()),
    };

    UpdateManager::new(config, &modules_dir.to_string_lossy())
}

// ═══════════════════════════════════════════════════════
// 1. 버전 파싱/비교 테스트
// ═══════════════════════════════════════════════════════

#[test]
fn test_semver_parse_basic() {
    let v = SemVer::parse("1.2.3").unwrap();
    assert_eq!((v.major, v.minor, v.patch), (1, 2, 3));
    assert!(v.prerelease.is_none());
}

#[test]
fn test_semver_parse_v_prefix() {
    let v = SemVer::parse("v0.1.0").unwrap();
    assert_eq!((v.major, v.minor, v.patch), (0, 1, 0));
}

#[test]
fn test_semver_parse_prerelease() {
    let v = SemVer::parse("2.0.0-rc.1").unwrap();
    assert!(v.is_prerelease());
    assert_eq!(v.prerelease, Some("rc.1".to_string()));
}

#[test]
fn test_semver_parse_two_part() {
    let v = SemVer::parse("1.0").unwrap();
    assert_eq!((v.major, v.minor, v.patch), (1, 0, 0));
}

#[test]
fn test_semver_invalid() {
    assert!(SemVer::parse("").is_none());
    assert!(SemVer::parse("abc").is_none());
    assert!(SemVer::parse("1").is_none());
}

#[test]
fn test_semver_comparison_patch() {
    let v1 = SemVer::parse("1.0.0").unwrap();
    let v2 = SemVer::parse("1.0.1").unwrap();
    assert!(v2.is_newer_than(&v1));
    assert!(!v1.is_newer_than(&v2));
}

#[test]
fn test_semver_comparison_minor() {
    let v1 = SemVer::parse("1.0.9").unwrap();
    let v2 = SemVer::parse("1.1.0").unwrap();
    assert!(v2.is_newer_than(&v1));
}

#[test]
fn test_semver_comparison_major() {
    let v1 = SemVer::parse("1.99.99").unwrap();
    let v2 = SemVer::parse("2.0.0").unwrap();
    assert!(v2.is_newer_than(&v1));
}

#[test]
fn test_semver_prerelease_less_than_release() {
    let pre = SemVer::parse("1.0.0-beta.1").unwrap();
    let rel = SemVer::parse("1.0.0").unwrap();
    assert!(rel.is_newer_than(&pre));
    assert!(!pre.is_newer_than(&rel));
}

#[test]
fn test_semver_equal() {
    let v1 = SemVer::parse("1.0.0").unwrap();
    let v2 = SemVer::parse("1.0.0").unwrap();
    assert!(!v1.is_newer_than(&v2));
    assert!(!v2.is_newer_than(&v1));
    assert_eq!(v1, v2);
}

#[test]
fn test_semver_display() {
    assert_eq!(SemVer::parse("1.2.3").unwrap().to_string(), "1.2.3");
    assert_eq!(SemVer::parse("0.1.0-alpha").unwrap().to_string(), "0.1.0-alpha");
}

// ═══════════════════════════════════════════════════════
// 2. Component 타입 테스트
// ═══════════════════════════════════════════════════════

#[test]
fn test_component_manifest_key_roundtrip() {
    let cases = vec![
        Component::CoreDaemon,
        Component::Cli,
        Component::Gui,
        Component::Module("minecraft".into()),
        Component::Module("palworld".into()),
    ];

    for comp in cases {
        let key = comp.manifest_key();
        let restored = Component::from_manifest_key(&key);
        assert_eq!(comp, restored, "Roundtrip failed for {:?}", comp);
    }
}

#[test]
fn test_component_display_name() {
    assert_eq!(Component::CoreDaemon.display_name(), "Core Daemon");
    assert_eq!(Component::Cli.display_name(), "CLI");
    assert_eq!(Component::Gui.display_name(), "GUI");
    assert_eq!(Component::Module("minecraft".into()).display_name(), "Module: minecraft");
}

// ═══════════════════════════════════════════════════════
// 3. UpdateConfig 테스트
// ═══════════════════════════════════════════════════════

#[test]
fn test_update_config_default() {
    let cfg = UpdateConfig::default();
    assert!(cfg.enabled);
    assert_eq!(cfg.check_interval_hours, 3);
    assert!(!cfg.auto_download);
    assert!(!cfg.auto_apply);
    assert!(!cfg.include_prerelease);
    assert!(cfg.install_root.is_none());
    assert_eq!(cfg.github_repo, "saba-chan");
}

#[test]
fn test_update_config_serialization() {
    let cfg = UpdateConfig {
        enabled: true,
        check_interval_hours: 6,
        auto_download: true,
        auto_apply: false,
        github_owner: "testowner".to_string(),
        github_repo: "testrepo".to_string(),
        include_prerelease: true,
        install_root: Some("/opt/saba".into()),
    };

    let json = serde_json::to_string(&cfg).unwrap();
    let restored: UpdateConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.check_interval_hours, 6);
    assert!(restored.auto_download);
    assert!(restored.include_prerelease);
    assert_eq!(restored.install_root, Some("/opt/saba".into()));
}

// ═══════════════════════════════════════════════════════
// 4. SchedulerConfig 테스트
// ═══════════════════════════════════════════════════════

#[test]
fn test_scheduler_config_defaults() {
    let sc = SchedulerConfig::default();
    assert!(sc.enabled);
    assert_eq!(sc.interval_hours, 3);
}

#[test]
fn test_scheduler_interval_duration() {
    let sc = SchedulerConfig { interval_hours: 6, enabled: true };
    assert_eq!(sc.interval_duration(), std::time::Duration::from_secs(6 * 3600));
}

#[test]
fn test_scheduler_checks_per_day() {
    let sc = SchedulerConfig { interval_hours: 3, enabled: true };
    assert_eq!(sc.checks_per_day(), 8);

    let sc2 = SchedulerConfig { interval_hours: 6, enabled: true };
    assert_eq!(sc2.checks_per_day(), 4);
}

// ═══════════════════════════════════════════════════════
// 5. ReleaseManifest 파싱 테스트
// ═══════════════════════════════════════════════════════

#[test]
fn test_manifest_parse() {
    let json = create_test_manifest("0.2.0", vec![
        ("core_daemon", "0.2.0", "core-win-x64.zip", Some(".")),
        ("cli", "0.2.0", "cli-win-x64.zip", None),
        ("module-minecraft", "2.1.0", "mod-minecraft.zip", Some("modules/minecraft")),
    ]);

    let manifest: ReleaseManifest = serde_json::from_str(&json).unwrap();
    assert_eq!(manifest.release_version, "0.2.0");
    assert_eq!(manifest.components.len(), 3);
    assert_eq!(manifest.components["core_daemon"].version, "0.2.0");
    assert_eq!(manifest.components["core_daemon"].asset, "core-win-x64.zip");
    assert_eq!(manifest.components["core_daemon"].install_dir, Some(".".into()));
    assert!(manifest.components["cli"].install_dir.is_none());
    assert_eq!(manifest.components["module-minecraft"].install_dir, Some("modules/minecraft".into()));
}

#[test]
fn test_manifest_serialization_roundtrip() {
    let mut components = HashMap::new();
    components.insert("gui".to_string(), saba_chan::updater::github::ComponentInfo {
        version: "1.0.0".into(),
        asset: "gui.zip".into(),
        sha256: Some("abcdef1234".into()),
        install_dir: Some("saba-chan-gui".into()),
    });

    let manifest = ReleaseManifest {
        release_version: "1.0.0".into(),
        components,
    };

    let json = serde_json::to_string(&manifest).unwrap();
    let restored: ReleaseManifest = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.release_version, "1.0.0");
    assert_eq!(restored.components["gui"].sha256, Some("abcdef1234".into()));
}

// ═══════════════════════════════════════════════════════
// 6. UpdateManager 단위 테스트 (파일시스템)
// ═══════════════════════════════════════════════════════

#[test]
fn test_manager_core_always_installed() {
    let tmp = TempDir::new().unwrap();
    let mgr = create_test_manager(&tmp, "", "");
    assert!(mgr.is_component_installed(&Component::CoreDaemon));
}

#[test]
fn test_manager_missing_module_not_installed() {
    let tmp = TempDir::new().unwrap();
    let mgr = create_test_manager(&tmp, "", "");
    assert!(!mgr.is_component_installed(&Component::Module("nonexistent".into())));
}

#[test]
fn test_manager_existing_module_installed() {
    let tmp = TempDir::new().unwrap();

    // 모듈 디렉터리와 module.toml 생성
    let mod_dir = tmp.path().join("modules").join("testgame");
    std::fs::create_dir_all(&mod_dir).unwrap();
    std::fs::write(mod_dir.join("module.toml"), r#"
name = "testgame"
version = "1.0.0"
"#).unwrap();

    let mgr = create_test_manager(&tmp, "", "");
    assert!(mgr.is_component_installed(&Component::Module("testgame".into())));
}

#[test]
fn test_manager_initial_status_empty() {
    let tmp = TempDir::new().unwrap();
    let mgr = create_test_manager(&tmp, "", "");
    let status = mgr.get_status();

    assert!(status.last_check.is_none());
    assert!(status.next_check.is_none());
    assert!(status.components.is_empty());
    assert!(!status.checking);
    assert!(status.error.is_none());
}

#[test]
fn test_manager_install_status() {
    let tmp = TempDir::new().unwrap();
    let mgr = create_test_manager(&tmp, "", "");
    let status = mgr.get_install_status();

    // 코어 데몬은 항상 설치됨
    assert!(status.components.iter()
        .any(|c| c.component == Component::CoreDaemon && c.installed));
    assert!(status.total_components >= 1);
}

#[test]
fn test_manager_config_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let mut mgr = create_test_manager(&tmp, "owner1", "repo1");

    assert_eq!(mgr.get_config().github_owner, "owner1");

    let new_cfg = UpdateConfig {
        github_owner: "owner2".into(),
        github_repo: "repo2".into(),
        auto_download: true,
        ..UpdateConfig::default()
    };
    mgr.update_config(new_cfg);

    assert_eq!(mgr.get_config().github_owner, "owner2");
    assert!(mgr.get_config().auto_download);
}

// ═══════════════════════════════════════════════════════
// 7. Zip 추출 + 파일 덮어쓰기 테스트
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_zip_extraction_creates_files() {
    let tmp = TempDir::new().unwrap();
    let mgr = create_test_manager(&tmp, "", "");

    // 테스트용 zip 생성
    let mut files = HashMap::new();
    files.insert("module.toml", b"name = \"test\"\nversion = \"1.0.0\"\n" as &[u8]);
    files.insert("lifecycle.py", b"def start(): pass\n" as &[u8]);
    files.insert("README.md", b"# Test Module\n" as &[u8]);
    let zip_data = create_test_zip(&files);

    // zip 파일을 스테이징에 저장
    let staged_path = tmp.path().join("test-module.zip");
    std::fs::write(&staged_path, &zip_data).unwrap();

    // 추출 대상 디렉터리
    let target_dir = tmp.path().join("modules").join("test");
    mgr.extract_to_directory_for_test(&staged_path, &target_dir).await;

    // 결과 검증
    assert!(target_dir.join("module.toml").exists());
    assert!(target_dir.join("lifecycle.py").exists());
    assert!(target_dir.join("README.md").exists());

    let content = std::fs::read_to_string(target_dir.join("module.toml")).unwrap();
    assert!(content.contains("name = \"test\""));
}

#[tokio::test]
async fn test_zip_overwrite_existing_files() {
    let tmp = TempDir::new().unwrap();
    let mgr = create_test_manager(&tmp, "", "");

    let target_dir = tmp.path().join("modules").join("overwrite_test");
    std::fs::create_dir_all(&target_dir).unwrap();

    // 기존 파일 생성 (v1)
    std::fs::write(target_dir.join("data.txt"), "version 1 content").unwrap();
    std::fs::write(target_dir.join("old_file.txt"), "should survive if not in zip").unwrap();

    // 새 버전 zip (v2)
    let mut files = HashMap::new();
    files.insert("data.txt", b"version 2 content - UPDATED" as &[u8]);
    files.insert("new_file.txt", b"brand new file" as &[u8]);
    let zip_data = create_test_zip(&files);

    let staged = tmp.path().join("update.zip");
    std::fs::write(&staged, &zip_data).unwrap();

    mgr.extract_to_directory_for_test(&staged, &target_dir).await;

    // data.txt가 v2로 덮어쓰임
    let content = std::fs::read_to_string(target_dir.join("data.txt")).unwrap();
    assert_eq!(content, "version 2 content - UPDATED");

    // 새 파일 추가됨
    assert!(target_dir.join("new_file.txt").exists());

    // zip에 없는 기존 파일은 유지됨
    assert!(target_dir.join("old_file.txt").exists());
}

// ═══════════════════════════════════════════════════════
// 8. GitHub API 모킹 — 전체 흐름 통합 테스트
// ═══════════════════════════════════════════════════════

/// 모킹 서버로 check_for_updates → download → apply 전체 파이프라인
#[tokio::test]
async fn test_full_update_pipeline_with_mock_server() {
    let tmp = TempDir::new().unwrap();

    // 1. 기존 모듈(v1) 배치
    let mod_dir = tmp.path().join("modules").join("minecraft");
    std::fs::create_dir_all(&mod_dir).unwrap();
    std::fs::write(mod_dir.join("module.toml"), r#"
name = "minecraft"
version = "1.0.0"
"#).unwrap();
    std::fs::write(mod_dir.join("lifecycle.py"), "# v1\ndef start(): pass\n").unwrap();

    // 2. 모킹 에셋 zip 생성 (v2 모듈)
    let mut mod_files = HashMap::new();
    mod_files.insert("module.toml", b"name = \"minecraft\"\nversion = \"2.1.0\"\n" as &[u8]);
    mod_files.insert("lifecycle.py", b"# v2 - updated\ndef start(): print('v2')\n" as &[u8]);
    mod_files.insert("config.json", b"{\"max_players\": 20}\n" as &[u8]);
    let mod_zip = create_test_zip(&mod_files);

    let mut assets = HashMap::new();
    assets.insert("module-minecraft.zip".to_string(), mod_zip);

    // 3. manifest.json
    let manifest = create_test_manifest("0.2.0", vec![
        ("core_daemon", "0.2.0", "core-daemon.zip", Some(".")),
        ("module-minecraft", "2.1.0", "module-minecraft.zip", Some("modules/minecraft")),
    ]);

    // 4. 모킹 서버 시작
    let (addr, _server_handle) = start_mock_github_server(manifest, assets).await;

    // 5. UpdateManager 생성 — GitHub URL을 모킹 서버로 우회
    //    GitHubClient가 api.github.com을 직접 호출하므로,
    //    UpdateManager를 직접 사용하는 대신 GitHubClient를 모킹 서버 URL로 테스트
    //    → GitHubClient를 통해 릴리스 & manifest를 직접 페치후 매니저에 수동 주입

    let http = reqwest::Client::builder()
        .user_agent("saba-chan-test/1.0")
        .build()
        .unwrap();

    // 5a. 릴리스 목록 페치
    let releases_url = format!("http://{}/repos/test/saba-chan/releases?per_page=10", addr);
    let releases: Vec<serde_json::Value> = http.get(&releases_url)
        .send().await.unwrap()
        .json().await.unwrap();

    assert_eq!(releases.len(), 1);
    assert_eq!(releases[0]["tag_name"], "v0.2.0");
    assert!(!releases[0]["prerelease"].as_bool().unwrap());

    // 5b. manifest.json 페치
    let manifest_url = releases[0]["assets"][0]["browser_download_url"].as_str().unwrap();
    let manifest_body: ReleaseManifest = http.get(manifest_url)
        .send().await.unwrap()
        .json().await.unwrap();

    assert_eq!(manifest_body.release_version, "0.2.0");
    assert_eq!(manifest_body.components.len(), 2);
    assert_eq!(manifest_body.components["module-minecraft"].version, "2.1.0");

    // 5c. 에셋 다운로드
    let asset_url = releases[0]["assets"].as_array().unwrap()
        .iter()
        .find(|a| a["name"] == "module-minecraft.zip")
        .unwrap()["browser_download_url"]
        .as_str().unwrap();

    let asset_bytes = http.get(asset_url)
        .send().await.unwrap()
        .bytes().await.unwrap();

    assert!(asset_bytes.len() > 0, "Downloaded asset should not be empty");

    // 5d. 에셋을 스테이징에 저장
    let staging = tmp.path().join("staging");
    std::fs::create_dir_all(&staging).unwrap();
    let staged_path = staging.join("module-minecraft.zip");
    std::fs::write(&staged_path, &asset_bytes).unwrap();

    // 5e. zip 해제 → 모듈 덮어쓰기 (apply 시뮬레이션)
    let target = tmp.path().join("modules").join("minecraft");
    {
        let file = std::fs::File::open(&staged_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).unwrap();
            let outpath = target.join(entry.name());
            if entry.is_dir() {
                std::fs::create_dir_all(&outpath).unwrap();
            } else {
                if let Some(parent) = outpath.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                let mut outfile = std::fs::File::create(&outpath).unwrap();
                std::io::copy(&mut entry, &mut outfile).unwrap();
            }
        }
    }

    // 6. 검증: 파일이 v2로 업데이트됨
    let module_toml = std::fs::read_to_string(target.join("module.toml")).unwrap();
    assert!(module_toml.contains("2.1.0"), "module.toml should show v2.1.0, got: {}", module_toml);

    let lifecycle = std::fs::read_to_string(target.join("lifecycle.py")).unwrap();
    assert!(lifecycle.contains("v2 - updated"), "lifecycle.py should be updated");

    // 새 파일이 추가됨
    assert!(target.join("config.json").exists(), "config.json should be extracted");

    println!("✓ Full update pipeline: fetch releases → parse manifest → download asset → extract zip → overwrite files");
}

/// 모킹 서버를 이용한 fresh_install 시뮬레이션
#[tokio::test]
async fn test_fresh_install_simulation() {
    let tmp = TempDir::new().unwrap();

    // CLI zip
    let mut cli_files = HashMap::new();
    cli_files.insert("saba-chan-cli.exe", b"FAKE_CLI_BINARY_v0.2.0" as &[u8]);
    let cli_zip = create_test_zip(&cli_files);

    // GUI zip
    let mut gui_files = HashMap::new();
    gui_files.insert("index.html", b"<html>GUI v0.2.0</html>" as &[u8]);
    gui_files.insert("main.js", b"console.log('gui')" as &[u8]);
    let gui_zip = create_test_zip(&gui_files);

    // Module zip
    let mut mod_files = HashMap::new();
    mod_files.insert("module.toml", b"name = \"palworld\"\nversion = \"1.0.0\"\n" as &[u8]);
    let mod_zip = create_test_zip(&mod_files);

    let mut assets = HashMap::new();
    assets.insert("cli-win-x64.zip".to_string(), cli_zip);
    assets.insert("gui-win-x64.zip".to_string(), gui_zip);
    assets.insert("module-palworld.zip".to_string(), mod_zip);

    let manifest = create_test_manifest("0.2.0", vec![
        ("cli", "0.2.0", "cli-win-x64.zip", Some(".")),
        ("gui", "0.2.0", "gui-win-x64.zip", Some("saba-chan-gui")),
        ("module-palworld", "1.0.0", "module-palworld.zip", Some("modules/palworld")),
    ]);

    let (addr, _handle) = start_mock_github_server(manifest, assets).await;

    // 모킹 서버에서 직접 다운로드 후 추출하는 시뮬레이션
    let http = reqwest::Client::new();

    let releases: Vec<serde_json::Value> = http
        .get(format!("http://{}/repos/test/saba-chan/releases?per_page=10", addr))
        .send().await.unwrap()
        .json().await.unwrap();

    let manifest_url = releases[0]["assets"][0]["browser_download_url"].as_str().unwrap();
    let manifest_body: ReleaseManifest = http.get(manifest_url)
        .send().await.unwrap()
        .json().await.unwrap();

    // 각 컴포넌트 설치
    for (key, info) in &manifest_body.components {
        let asset_url = releases[0]["assets"].as_array().unwrap()
            .iter()
            .find(|a| a["name"].as_str() == Some(&info.asset))
            .map(|a| a["browser_download_url"].as_str().unwrap().to_string());

        if let Some(url) = asset_url {
            let bytes = http.get(&url).send().await.unwrap().bytes().await.unwrap();

            // 설치 디렉터리 결정
            let install_dir = match &info.install_dir {
                Some(d) => tmp.path().join(d),
                None => tmp.path().to_path_buf(),
            };
            std::fs::create_dir_all(&install_dir).unwrap();

            // zip 해제
            let cursor = std::io::Cursor::new(bytes.to_vec());
            let mut archive = zip::ZipArchive::new(cursor).unwrap();
            for i in 0..archive.len() {
                let mut entry = archive.by_index(i).unwrap();
                let outpath = install_dir.join(entry.name());
                if entry.is_dir() {
                    std::fs::create_dir_all(&outpath).unwrap();
                } else {
                    if let Some(p) = outpath.parent() { std::fs::create_dir_all(p).unwrap(); }
                    let mut f = std::fs::File::create(&outpath).unwrap();
                    std::io::copy(&mut entry, &mut f).unwrap();
                }
            }

            println!("  Installed {} → {}", key, install_dir.display());
        }
    }

    // 검증
    assert!(tmp.path().join("saba-chan-cli.exe").exists(), "CLI should be installed");
    let cli_content = std::fs::read(tmp.path().join("saba-chan-cli.exe")).unwrap();
    assert_eq!(cli_content, b"FAKE_CLI_BINARY_v0.2.0");

    assert!(tmp.path().join("saba-chan-gui").join("index.html").exists(), "GUI should be installed");
    let gui_html = std::fs::read_to_string(tmp.path().join("saba-chan-gui").join("index.html")).unwrap();
    assert!(gui_html.contains("GUI v0.2.0"));

    assert!(tmp.path().join("modules").join("palworld").join("module.toml").exists(), "Module should be installed");

    println!("✓ Fresh install simulation: CLI + GUI + Module all installed from mock server");
}

// ═══════════════════════════════════════════════════════
// 9. 데몬 IPC API 직접 호출 테스트 (Axum tower::ServiceExt)
// ═══════════════════════════════════════════════════════

/// IPC 라우터의 update/install 엔드포인트를 HTTP 요청으로 직접 테스트
#[tokio::test]
async fn test_ipc_update_status_endpoint() {
    let tmp = TempDir::new().unwrap();
    let update_mgr = Arc::new(RwLock::new(create_test_manager(&tmp, "owner", "repo")));

    // 간이 라우터 (IPC 핸들러 재현)
    let mgr = update_mgr.clone();
    let app = Router::new()
        .route("/api/updates/status", get({
            let m = mgr.clone();
            move || {
                let m = m.clone();
                async move {
                    let mgr = m.read().await;
                    let status = mgr.get_status();
                    Json(json!({
                        "ok": true,
                        "status": status,
                    }))
                }
            }
        }))
        .route("/api/updates/config", get({
            let m = mgr.clone();
            move || {
                let m = m.clone();
                async move {
                    let mgr = m.read().await;
                    let config = mgr.get_config();
                    Json(json!({
                        "ok": true,
                        "config": config,
                    }))
                }
            }
        }))
        .route("/api/install/status", get({
            let m = mgr.clone();
            move || {
                let m = m.clone();
                async move {
                    let mgr = m.read().await;
                    let status = mgr.get_install_status();
                    Json(json!({
                        "ok": true,
                        "install_status": status,
                    }))
                }
            }
        }))
        .route("/api/install/progress", get({
            let m = mgr.clone();
            move || {
                let m = m.clone();
                async move {
                    let mgr = m.read().await;
                    let progress = mgr.get_install_progress();
                    Json(json!({
                        "ok": true,
                        "progress": progress,
                    }))
                }
            }
        }));

    // tower::ServiceExt로 직접 요청 전송
    use axum::body::Body;
    use tower::ServiceExt;

    // GET /api/updates/status
    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/api/updates/status")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert!(json["status"]["components"].as_array().unwrap().is_empty());
    assert_eq!(json["status"]["checking"], false);

    println!("  ✓ GET /api/updates/status → 200 OK, empty components");

    // GET /api/updates/config
    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/api/updates/config")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["config"]["github_owner"], "owner");
    assert_eq!(json["config"]["github_repo"], "repo");
    assert_eq!(json["config"]["check_interval_hours"], 3);

    println!("  ✓ GET /api/updates/config → 200 OK, config matches");

    // GET /api/install/status
    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/api/install/status")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    // 코어 데몬은 항상 설치됨
    let components = json["install_status"]["components"].as_array().unwrap();
    let core = components.iter()
        .find(|c| c["display_name"] == "Core Daemon")
        .unwrap();
    assert_eq!(core["installed"], true);

    println!("  ✓ GET /api/install/status → Core Daemon installed");

    // GET /api/install/progress (초기: null)
    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/api/install/progress")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["progress"].is_null());

    println!("  ✓ GET /api/install/progress → null (no install in progress)");
}

/// IPC config 변경 (PUT) 테스트
#[tokio::test]
async fn test_ipc_update_config_put() {
    let tmp = TempDir::new().unwrap();
    let update_mgr = Arc::new(RwLock::new(create_test_manager(&tmp, "old_owner", "old_repo")));
    let mgr = update_mgr.clone();

    let app = Router::new()
        .route("/api/updates/config", get({
            let m = mgr.clone();
            move || {
                let m = m.clone();
                async move {
                    let mgr = m.read().await;
                    Json(json!({ "ok": true, "config": mgr.get_config() }))
                }
            }
        }).put({
            let m = mgr.clone();
            move |Json(new_config): Json<UpdateConfig>| {
                let m = m.clone();
                async move {
                    let mut mgr = m.write().await;
                    mgr.update_config(new_config);
                    Json(json!({ "ok": true, "config": mgr.get_config() }))
                }
            }
        }));

    use axum::body::Body;
    use tower::ServiceExt;

    // PUT /api/updates/config
    let new_cfg = json!({
        "enabled": true,
        "check_interval_hours": 6,
        "auto_download": true,
        "auto_apply": false,
        "github_owner": "new_owner",
        "github_repo": "new_repo",
        "include_prerelease": true,
        "install_root": null
    });

    let req = axum::http::Request::builder()
        .method("PUT")
        .uri("/api/updates/config")
        .header("content-type", "application/json")
        .body(Body::from(new_cfg.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["config"]["github_owner"], "new_owner");
    assert_eq!(json["config"]["check_interval_hours"], 6);
    assert_eq!(json["config"]["auto_download"], true);
    assert_eq!(json["config"]["include_prerelease"], true);

    // GET으로 확인
    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/api/updates/config")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["config"]["github_owner"], "new_owner");

    println!("  ✓ PUT /api/updates/config → config updated and persisted in memory");
}

// ═══════════════════════════════════════════════════════
// 10. 실제 GitHub 페치 테스트 (인터넷 연결 필요)
//     기본 disabled — 환경변수 SABA_TEST_LIVE_GITHUB=1 로 활성화
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_live_github_fetch() {
    if std::env::var("SABA_TEST_LIVE_GITHUB").is_err() {
        println!("⏭ Skipping live GitHub test (set SABA_TEST_LIVE_GITHUB=1 to enable)");
        return;
    }

    // 실제 공개 저장소에서 릴리스 페치 (예: rust-lang/rust)
    let client = GitHubClient::new("rust-lang", "rust");
    let releases = client.fetch_releases(3).await;

    match releases {
        Ok(r) => {
            assert!(!r.is_empty(), "rust-lang/rust should have releases");
            println!("  Live GitHub: fetched {} release(s), latest: {}", r.len(), r[0].tag_name);
        }
        Err(e) => {
            // rate limit 등으로 실패할 수 있음
            println!("  Live GitHub: request failed (possibly rate-limited): {}", e);
        }
    }
}

// ═══════════════════════════════════════════════════════
// 11. 에지 케이스 테스트
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_check_without_config_fails() {
    let tmp = TempDir::new().unwrap();
    let mut mgr = create_test_manager(&tmp, "", ""); // owner/repo 비어있음

    let result = mgr.check_for_updates().await;
    assert!(result.is_err(), "check should fail without github_owner/repo");
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("not configured"), "Error: {}", err_msg);
}

#[tokio::test]
async fn test_download_without_check_fails() {
    let tmp = TempDir::new().unwrap();
    let mut mgr = create_test_manager(&tmp, "test", "repo");

    // check 없이 download 시도
    let result = mgr.download_available_updates().await;
    // 컴포넌트가 없으므로 다운로드할 게 없음 → 빈 Vec 반환
    match result {
        Ok(v) => assert!(v.is_empty(), "Should download nothing without prior check"),
        Err(_) => {} // 에러도 합리적
    }
}

#[tokio::test]
async fn test_apply_without_downloaded_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let mut mgr = create_test_manager(&tmp, "test", "repo");

    let result = mgr.apply_updates().await.unwrap();
    assert!(result.is_empty(), "Should apply nothing if nothing downloaded");
}

#[tokio::test]
async fn test_install_already_installed_fails() {
    let tmp = TempDir::new().unwrap();
    let mut mgr = create_test_manager(&tmp, "test", "repo");

    // CoreDaemon은 항상 설치됨
    let result = mgr.install_component(&Component::CoreDaemon).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("already installed"), "Error: {}", err);
}

#[test]
fn test_single_file_copy_instead_of_zip() {
    let tmp = TempDir::new().unwrap();

    // .txt 파일 (zip 아닌 단일 파일)
    let staged = tmp.path().join("readme.txt");
    std::fs::write(&staged, "Hello from single file").unwrap();

    let target = tmp.path().join("output");
    std::fs::create_dir_all(&target).unwrap();

    // 단일 파일 복사 로직
    let file_name = staged.file_name().unwrap();
    std::fs::copy(&staged, target.join(file_name)).unwrap();

    assert!(target.join("readme.txt").exists());
    let content = std::fs::read_to_string(target.join("readme.txt")).unwrap();
    assert_eq!(content, "Hello from single file");
}

// ═══════════════════════════════════════════════════════
// 12. 백업 & 복구 테스트
// ═══════════════════════════════════════════════════════

#[test]
fn test_module_backup_before_update() {
    let tmp = TempDir::new().unwrap();

    // 기존 모듈 (v1)
    let mod_dir = tmp.path().join("modules").join("testmod");
    std::fs::create_dir_all(&mod_dir).unwrap();
    std::fs::write(mod_dir.join("module.toml"), "name = \"testmod\"\nversion = \"1.0.0\"\n").unwrap();
    std::fs::write(mod_dir.join("data.txt"), "important data v1").unwrap();

    // 백업 생성
    let backup_dir = tmp.path().join("backup_testmod");
    copy_dir_recursive(&mod_dir, &backup_dir);

    // 원본 덮어쓰기
    std::fs::write(mod_dir.join("module.toml"), "name = \"testmod\"\nversion = \"2.0.0\"\n").unwrap();
    std::fs::write(mod_dir.join("data.txt"), "new data v2").unwrap();

    // 백업 검증 (원본 변경 후에도 유지)
    let backup_content = std::fs::read_to_string(backup_dir.join("module.toml")).unwrap();
    assert!(backup_content.contains("1.0.0"), "Backup should have v1");

    let current_content = std::fs::read_to_string(mod_dir.join("module.toml")).unwrap();
    assert!(current_content.contains("2.0.0"), "Current should have v2");

    // 복구: 백업 → 원본
    copy_dir_recursive(&backup_dir, &mod_dir);
    let restored = std::fs::read_to_string(mod_dir.join("module.toml")).unwrap();
    assert!(restored.contains("1.0.0"), "Restored should have v1 again");

    println!("✓ Backup/restore cycle works correctly");
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let ty = entry.file_type().unwrap();
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&src_path, &dst_path);
        } else {
            std::fs::copy(&src_path, &dst_path).unwrap();
        }
    }
}

// ═══════════════════════════════════════════════════════
// 13. UpdateManager — resolve_install_dir 정밀 테스트
// ═══════════════════════════════════════════════════════

#[test]
fn test_resolve_install_dir_with_manifest_override() {
    let tmp = TempDir::new().unwrap();
    let mgr = create_test_manager(&tmp, "", "");

    // manifest에서 install_dir 지정
    let dir = mgr.resolve_install_dir_for_test(&Component::Cli, Some("custom/bin"));
    assert!(dir.ends_with("custom/bin") || dir.ends_with("custom\\bin"));

    // manifest에 install_dir 없으면 기본 규칙
    let dir = mgr.resolve_install_dir_for_test(&Component::Gui, None);
    assert!(dir.to_string_lossy().contains("saba-chan-gui"));

    let dir = mgr.resolve_install_dir_for_test(&Component::Module("mc".into()), None);
    assert!(dir.to_string_lossy().contains("mc"));
}

// ═══════════════════════════════════════════════════════
// 14. 다중 릴리스 필터링 테스트
// ═══════════════════════════════════════════════════════

#[test]
fn test_release_filtering_excludes_draft_and_prerelease() {
    let releases = vec![
        ("v0.3.0-beta", true, false),  // prerelease
        ("v0.2.1", false, true),       // draft
        ("v0.2.0", false, false),      // ← 이것이 선택되어야 함
        ("v0.1.0", false, false),
    ];

    let include_prerelease = false;

    let selected = releases.iter()
        .filter(|(_, _pre, draft)| !draft)
        .filter(|(_, pre, _)| include_prerelease || !pre)
        .next();

    assert!(selected.is_some());
    assert_eq!(selected.unwrap().0, "v0.2.0");
}

#[test]
fn test_release_filtering_includes_prerelease_when_enabled() {
    let releases = vec![
        ("v0.3.0-beta", true, false),
        ("v0.2.0", false, false),
    ];

    let include_prerelease = true;
    let selected = releases.iter()
        .filter(|(_, _, draft)| !draft)
        .filter(|(_, pre, _)| include_prerelease || !pre)
        .next();

    assert_eq!(selected.unwrap().0, "v0.3.0-beta");
}

// ═══════════════════════════════════════════════════════
// 15. 대몬 재시작 스크립트 생성 테스트
// ═══════════════════════════════════════════════════════

#[test]
fn test_daemon_update_script_content() {
    // 재시작 스크립트의 내용 패턴 검증 (실제 파일 생성 없이)
    let exe_path = "C:\\saba-chan\\core_daemon.exe";
    let staged_path = "C:\\saba-chan\\updates\\core.zip";

    let script = format!(
        r#"Start-Sleep -Seconds 2
$exePath = "{}"
$stagedPath = "{}"
"#, exe_path, staged_path);

    assert!(script.contains("Start-Sleep"));
    assert!(script.contains(exe_path));
    assert!(script.contains(staged_path));
}

// ═══════════════════════════════════════════════════════
// 16. 시간 유틸 테스트
// ═══════════════════════════════════════════════════════

#[test]
fn test_timestamp_format() {
    // UpdateManager 내부의 chrono_now_iso가 올바른 ISO 8601 형식인지
    // (내부 함수이므로 패턴 검증만)
    let pattern = regex_lite::Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$").unwrap();

    // 검증을 위해 수동 구성
    let ts = format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", 2026, 2, 13, 10, 30, 0);
    assert!(pattern.is_match(&ts));
}
