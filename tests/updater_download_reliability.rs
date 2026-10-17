//! ═══════════════════════════════════════════════════════════════════
//! P0-9 Sc7: Updater 다운로드 신뢰성 (네트워크 단절 / 부분 응답 / 해시 불일치)
//! ═══════════════════════════════════════════════════════════════════
//!
//! 분류: **Component Integration** — 별도 OS 프로세스는 spawn하지 않으나,
//! 실제 TCP 소켓을 통해 mock HTTP 서버와 reqwest 클라이언트가 통신.
//!
//! ## 검증 시나리오 (Sc7 자동화)
//! 1. **정상 다운로드 + 해시 검증 OK**
//! 2. **HTTP 5xx**: 명확한 에러 + dest 파일 삭제됨
//! 3. **연결 단절(중간 절단)**: 스트림 읽기 실패 → dest 파일 삭제됨
//! 4. **해시 불일치**: 파일 다운로드 완료되더라도 검증 실패 시 파일 삭제됨

use saba_chan_updater_lib::integrity::download::download_verified;
use saba_chan_updater_lib::integrity::hash::FileHash;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

fn pick_free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let p = l.local_addr().unwrap().port();
    drop(l);
    p
}

fn write_request_response(mut s: TcpStream, status_line: &str, body: &[u8]) {
    let mut buf = [0u8; 2048];
    let _ = s.read(&mut buf);
    let header = format!(
        "{}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
        status_line,
        body.len()
    );
    let _ = s.write_all(header.as_bytes());
    let _ = s.write_all(body);
    let _ = s.flush();
}

fn write_request_response_then_drop(mut s: TcpStream, status_line: &str, partial_body: &[u8], promised_len: usize) {
    let mut buf = [0u8; 2048];
    let _ = s.read(&mut buf);
    let header = format!(
        "{}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
        status_line, promised_len
    );
    let _ = s.write_all(header.as_bytes());
    let _ = s.write_all(partial_body);
    let _ = s.flush();
    // 본문을 다 보내지 않은 채 drop → 클라이언트 측은 EOF를 받지만 Content-Length 부족
    drop(s);
}

#[tokio::test]
async fn download_verified_succeeds_with_correct_hash() {
    let port = pick_free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();

    let body = b"saba-chan-updater-payload-A".to_vec();
    let body_for_thread = body.clone();
    thread::spawn(move || {
        if let Ok((s, _)) = listener.accept() {
            write_request_response(s, "HTTP/1.1 200 OK", &body_for_thread);
        }
    });
    thread::sleep(Duration::from_millis(50));

    let mut hasher = Sha256::new();
    hasher.update(&body);
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&hasher.finalize());
    let expected = FileHash::from_bytes(arr);

    let dest = tempfile::tempdir().unwrap();
    let dest_file = dest.path().join("payload.bin");
    let url = format!("http://127.0.0.1:{}/file", port);
    let client = reqwest::Client::new();

    let result = download_verified(&client, &url, &dest_file, Some(&expected), None).await;
    assert!(result.is_ok(), "정상 다운로드 + 해시 일치 시 Ok: {:?}", result.err());
    assert!(dest_file.exists(), "성공 시 파일이 dest에 남아 있어야 함");
}

#[tokio::test]
async fn download_verified_deletes_file_on_hash_mismatch() {
    let port = pick_free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();

    let body = b"actual-payload".to_vec();
    let body_for_thread = body.clone();
    thread::spawn(move || {
        if let Ok((s, _)) = listener.accept() {
            write_request_response(s, "HTTP/1.1 200 OK", &body_for_thread);
        }
    });
    thread::sleep(Duration::from_millis(50));

    // 잘못된 기대 해시 (다른 데이터로 계산)
    let mut hasher = Sha256::new();
    hasher.update(b"different-payload");
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&hasher.finalize());
    let wrong_expected = FileHash::from_bytes(arr);

    let dest = tempfile::tempdir().unwrap();
    let dest_file = dest.path().join("payload.bin");
    let url = format!("http://127.0.0.1:{}/file", port);
    let client = reqwest::Client::new();

    let result = download_verified(&client, &url, &dest_file, Some(&wrong_expected), None).await;
    assert!(result.is_err(), "해시 불일치는 에러여야 함");
    assert!(
        !dest_file.exists(),
        "해시 불일치 시 다운로드 파일은 삭제되어야 함 — 반쯤 쓴 파일 방지"
    );
}

#[tokio::test]
async fn download_verified_returns_error_on_http_5xx() {
    let port = pick_free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();

    thread::spawn(move || {
        if let Ok((s, _)) = listener.accept() {
            write_request_response(s, "HTTP/1.1 500 Internal Server Error", b"err");
        }
    });
    thread::sleep(Duration::from_millis(50));

    let dest = tempfile::tempdir().unwrap();
    let dest_file = dest.path().join("payload.bin");
    let url = format!("http://127.0.0.1:{}/file", port);
    let client = reqwest::Client::new();

    let result = download_verified(&client, &url, &dest_file, None, None).await;
    assert!(result.is_err(), "5xx는 에러여야 함");
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("500"), "에러에 status 포함: {msg}");
}

#[tokio::test]
async fn download_verified_cleans_up_on_connection_drop() {
    let port = pick_free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();

    thread::spawn(move || {
        if let Ok((s, _)) = listener.accept() {
            // Content-Length 1000으로 약속하고 100바이트만 보낸 뒤 연결 종료 (네트워크 단절 시뮬레이션)
            let partial = vec![b'x'; 100];
            write_request_response_then_drop(s, "HTTP/1.1 200 OK", &partial, 1000);
        }
    });
    thread::sleep(Duration::from_millis(50));

    let dest = tempfile::tempdir().unwrap();
    let dest_file = dest.path().join("payload.bin");
    let url = format!("http://127.0.0.1:{}/file", port);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();

    // 기대 해시는 임의 — 어차피 끝까지 읽지 못해 해시 비교 전에 실패함
    let mut arr = [0u8; 32];
    arr[0] = 0xAB;
    let dummy_expected = FileHash::from_bytes(arr);

    let result = download_verified(&client, &url, &dest_file, Some(&dummy_expected), None).await;
    // 두 가지 결과 중 하나여야 한다:
    //   (a) 스트림 읽기 실패로 즉시 Download 에러
    //   (b) 끝까지 읽힌 듯 보이고 해시 불일치 — 둘 다 정상 결함 처리
    assert!(result.is_err(), "부분 응답은 에러여야 함");
    assert!(
        !dest_file.exists(),
        "연결 단절/해시 불일치 시 dest 파일은 삭제되어야 함"
    );
}
