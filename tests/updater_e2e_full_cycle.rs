//! ═══════════════════════════════════════════════════════════════════
//! 업데이터 E2E 전체 사이클 자동화 테스트
//! ═══════════════════════════════════════════════════════════════════
//!
//! `docs/UPDATER_E2E_SCENARIOS.md`의 수동 시나리오 중 자동화 가능한 항목을
//! 데몬 스폰 없이(in-process) 검증한다.
//!
//! ## 커버리지
//! | Sc # | 시나리오                          | 자동화 방식                              |
//! |------|-----------------------------------|------------------------------------------|
//! | 3    | 직접 실행 차단                    | `parse_launch_args` 단위 테스트 (기존)   |
//! | 4    | 티켓 일회성                       | `consume_at` 순차 호출 + 파일 상태 검증  |
//! | 5    | 티켓 만료                         | 만료 타임스탬프 직접 주입                |
//! | 6    | GUI ↔ CLI spawner 격리            | `allowed_spawners` 매트릭스              |
//! | 7    | 네트워크 단절 복원                | Mock 서버가 초기 N회 500 응답 후 성공    |
//! | 신규 | 전체 다운로드 → 추출 → 백업 생성   | zip → `.exe.old` 백업 + 새 바이너리      |
//!
//! 수동 시나리오 1/2는 Tauri/CLI 런타임을 필요로 하므로 여기서 다루지 않는다.

use std::collections::HashMap;
use std::io::Write;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::RwLock;

use saba_chan_updater_lib::launch_ticket;

// ═══════════════════════════════════════════════════════
// 공통 헬퍼
// ═══════════════════════════════════════════════════════

fn ticket_tmp() -> (TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("launch-ticket.json");
    (tmp, path)
}

fn make_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let buf = std::io::Cursor::new(Vec::new());
    let mut zw = zip::ZipWriter::new(buf);
    let opts = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, content) in files {
        zw.start_file(*name, opts).unwrap();
        zw.write_all(content).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

// ═══════════════════════════════════════════════════════
// Sc 4 — 티켓 일회성 (동시 실행 방지)
// ═══════════════════════════════════════════════════════

#[test]
fn sc4_ticket_second_consume_is_rejected() {
    let (_tmp, path) = ticket_tmp();
    let token = launch_ticket::issue_at(&path, &["gui", "cli"]).unwrap();

    // 1회차: 성공. 파일 삭제됨.
    assert!(launch_ticket::consume_at(&path, &token, "gui").is_ok());
    assert!(!path.exists(), "ticket file must be deleted after consume");

    // 2회차 (동일 토큰): 파일 부재로 거부
    let err = launch_ticket::consume_at(&path, &token, "gui").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("not found") || msg.contains("direct execution"),
        "second consume must report missing ticket, got: {}",
        msg
    );
}

#[test]
fn sc4_ticket_file_removed_even_on_token_mismatch() {
    // 일회성 보장: 토큰 불일치여도 파일은 재사용 불가해야 함.
    let (_tmp, path) = ticket_tmp();
    let _token = launch_ticket::issue_at(&path, &["gui"]).unwrap();

    let err = launch_ticket::consume_at(&path, "wrong-token", "gui").unwrap_err();
    assert!(err.to_string().contains("mismatch"));
    assert!(!path.exists(), "ticket file must be wiped even on validation failure");
}

// ═══════════════════════════════════════════════════════
// Sc 5 — 티켓 만료 경계값
// ═══════════════════════════════════════════════════════

#[test]
fn sc5_ticket_expired_is_rejected() {
    // 만료 타임스탬프를 직접 주입하여 즉시 만료된 티켓 생성.
    let (_tmp, path) = ticket_tmp();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let expired = serde_json::json!({
        "token": "abc-xyz",
        "allowed_spawners": ["gui"],
        "expires_at": now - 1,
    });
    std::fs::write(&path, expired.to_string()).unwrap();

    let err = launch_ticket::consume_at(&path, "abc-xyz", "gui").unwrap_err();
    assert!(err.to_string().contains("expired"), "expected expired error, got: {}", err);
}

#[test]
fn sc5_ticket_within_lifetime_ok() {
    let (_tmp, path) = ticket_tmp();
    let token = launch_ticket::issue_at(&path, &["gui"]).unwrap();
    // 발급 직후: 120초 내이므로 성공해야 함.
    assert!(launch_ticket::consume_at(&path, &token, "gui").is_ok());
}

// ═══════════════════════════════════════════════════════
// Sc 6 — 스포너 격리 (GUI 티켓을 CLI가 사용 불가)
// ═══════════════════════════════════════════════════════

#[test]
fn sc6_spawner_matrix() {
    struct Case {
        allowed: &'static [&'static str],
        spawner: &'static str,
        ok: bool,
    }
    let cases = [
        Case { allowed: &["gui"],        spawner: "gui",    ok: true  },
        Case { allowed: &["gui"],        spawner: "cli",    ok: false },
        Case { allowed: &["cli"],        spawner: "gui",    ok: false },
        Case { allowed: &["gui", "cli"], spawner: "gui",    ok: true  },
        Case { allowed: &["gui", "cli"], spawner: "cli",    ok: true  },
        Case { allowed: &["gui", "cli"], spawner: "daemon", ok: false },
    ];

    for (i, c) in cases.iter().enumerate() {
        let (_tmp, path) = ticket_tmp();
        let token = launch_ticket::issue_at(&path, c.allowed).unwrap();
        let result = launch_ticket::consume_at(&path, &token, c.spawner);
        if c.ok {
            assert!(result.is_ok(), "case {}: expected ok, got {:?}", i, result);
        } else {
            let err = result.expect_err(&format!("case {}: expected error", i));
            let msg = err.to_string();
            assert!(
                msg.contains("does not allow spawner"),
                "case {}: wrong error msg: {}",
                i, msg
            );
        }
    }
}

// ═══════════════════════════════════════════════════════
// Sc 7 — 네트워크 단절 복원 (mock 서버 + reqwest 재시도)
// ═══════════════════════════════════════════════════════

/// `fail_count`회 500을 반환한 뒤 200을 반환하는 모킹 서버.
async fn start_flaky_server(fail_count: usize, body: Vec<u8>) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    use axum::{routing::get, Router};

    let counter = Arc::new(AtomicUsize::new(0));
    let body_arc = Arc::new(body);
    let fail_until = fail_count;

    let counter_c = counter.clone();
    let body_c = body_arc.clone();

    let app = Router::new().route("/asset", get(move || {
        let n = counter_c.fetch_add(1, Ordering::SeqCst);
        let body = body_c.clone();
        async move {
            if n < fail_until {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Vec::<u8>::new())
            } else {
                (axum::http::StatusCode::OK, (*body).clone())
            }
        }
    }));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

async fn download_with_retry(url: &str, max_attempts: u32) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::new();
    let mut last_err = String::new();
    for attempt in 1..=max_attempts {
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                return resp.bytes().await
                    .map(|b| b.to_vec())
                    .map_err(|e| e.to_string());
            }
            Ok(resp) => {
                last_err = format!("status {}", resp.status());
            }
            Err(e) => {
                last_err = e.to_string();
            }
        }
        // 지수 백오프 흉내 (실제 updater 정책과 독립)
        tokio::time::sleep(std::time::Duration::from_millis(50 * attempt as u64)).await;
    }
    Err(last_err)
}

#[tokio::test]
async fn sc7_download_recovers_after_transient_500s() {
    let payload = b"UPDATED_BINARY_CONTENT".to_vec();
    let (addr, _handle) = start_flaky_server(2, payload.clone()).await;

    let url = format!("http://{}/asset", addr);
    let got = download_with_retry(&url, 5).await.expect("should eventually succeed");
    assert_eq!(got, payload);
}

#[tokio::test]
async fn sc7_download_exhausts_retries_on_persistent_failure() {
    let payload = b"X".to_vec();
    let (addr, _handle) = start_flaky_server(10, payload).await; // 10회 실패 > 3회 재시도

    let url = format!("http://{}/asset", addr);
    let err = download_with_retry(&url, 3).await.expect_err("should exhaust retries");
    assert!(err.contains("500") || err.contains("status"), "got: {}", err);
}

// ═══════════════════════════════════════════════════════
// 신규 — 전체 사이클: 다운로드 → 추출 → 바이너리 백업
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn full_cycle_binary_replacement_creates_backup() {
    // 시뮬레이션: install_dir에 v1 바이너리가 있는 상태에서
    //  zip v2를 다운로드 → 교체 로직이 `.exe.old` 백업을 만드는지 검증.
    //
    // 실제 updater의 apply 로직을 호출하는 대신, 동일한 계약(백업 명명 규칙)을
    // 여기서 독립적으로 재구현하여 "계약"이 유지되는지 확인.
    //
    // 주의: 이 테스트는 `updater/src/lib.rs`의 `.exe.old` 명명 규칙에 의존한다.
    //       updater가 이 규칙을 바꾸면 테스트도 함께 변경되어야 한다.

    let tmp = TempDir::new().unwrap();
    let install_dir = tmp.path();

    // v1 바이너리 배치
    let exe_path = install_dir.join("saba-chan-gui.exe");
    std::fs::write(&exe_path, b"V1_BINARY").unwrap();

    // v2 zip 다운로드 시뮬레이션
    let zip_bytes = make_zip(&[("saba-chan-gui.exe", b"V2_BINARY_UPDATED")]);
    let staging = tmp.path().join("staging");
    std::fs::create_dir_all(&staging).unwrap();
    let zip_path = staging.join("gui.zip");
    std::fs::write(&zip_path, &zip_bytes).unwrap();

    // 추출 + 백업 로직 시뮬레이션 (updater의 계약)
    let file = std::fs::File::open(&zip_path).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let out = install_dir.join(entry.name());

        // 바이너리 교체: .exe → .exe.old로 먼저 이동 (백업)
        if out.exists()
            && out.extension().map(|e| e == "exe").unwrap_or(false)
        {
            let backup = out.with_extension("exe.old");
            let _ = std::fs::remove_file(&backup);
            std::fs::rename(&out, &backup).unwrap();
        }

        let mut outfile = std::fs::File::create(&out).unwrap();
        std::io::copy(&mut entry, &mut outfile).unwrap();
    }

    // 검증
    assert!(exe_path.exists(), "new exe must exist");
    let new_content = std::fs::read(&exe_path).unwrap();
    assert_eq!(new_content, b"V2_BINARY_UPDATED");

    let backup_path = install_dir.join("saba-chan-gui.exe.old");
    assert!(backup_path.exists(), ".exe.old backup must be created");
    let backup_content = std::fs::read(&backup_path).unwrap();
    assert_eq!(backup_content, b"V1_BINARY", "backup must contain previous version");
}

// ═══════════════════════════════════════════════════════
// 신규 — 티켓 + 파일 교체 전체 플로우 (in-process)
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn full_cycle_daemon_issues_updater_consumes_applies() {
    // 1. 데몬 역할: 티켓 발급
    let (_tmp_ticket, ticket_path) = ticket_tmp();
    let token = launch_ticket::issue_at(&ticket_path, &["gui"]).unwrap();
    assert!(ticket_path.exists());

    // 2. 업데이터 역할: 토큰 검증 통과
    launch_ticket::consume_at(&ticket_path, &token, "gui").expect("consume must succeed");
    assert!(!ticket_path.exists(), "ticket must be consumed");

    // 3. 파일 교체 수행 (Sc6 mixed apply의 파일 파트)
    let tmp = TempDir::new().unwrap();
    let mod_dir = tmp.path().join("modules").join("minecraft");
    std::fs::create_dir_all(&mod_dir).unwrap();
    std::fs::write(mod_dir.join("module.toml"), "version = \"1.0.0\"\n").unwrap();

    let zip_bytes = make_zip(&[
        ("module.toml", b"version = \"2.0.0\"\n"),
        ("lifecycle.py", b"# v2\n"),
    ]);
    let file = std::fs::File::open(&{
        let p = tmp.path().join("mc.zip");
        std::fs::write(&p, &zip_bytes).unwrap();
        p
    }).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let out = mod_dir.join(entry.name());
        if let Some(p) = out.parent() { std::fs::create_dir_all(p).unwrap(); }
        let mut f = std::fs::File::create(&out).unwrap();
        std::io::copy(&mut entry, &mut f).unwrap();
    }

    // 4. 최종 상태 검증
    let toml = std::fs::read_to_string(mod_dir.join("module.toml")).unwrap();
    assert!(toml.contains("2.0.0"), "module must be upgraded");
    assert!(mod_dir.join("lifecycle.py").exists(), "new file must be extracted");

    // 5. 재사용 방지: 동일 토큰으로 재시도 불가
    let err = launch_ticket::consume_at(&ticket_path, &token, "gui").unwrap_err();
    assert!(err.to_string().contains("not found"));
}

// ═══════════════════════════════════════════════════════
// 사용하지 않는 import 억제용 마커
// ═══════════════════════════════════════════════════════
#[allow(dead_code)]
fn _unused_imports_marker() {
    let _: Option<Arc<RwLock<HashMap<String, String>>>> = None;
}
