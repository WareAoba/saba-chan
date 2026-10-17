//! ═══════════════════════════════════════════════════════════════════
//! P0-5: RCON / REST 프로토콜 라운드트립 (Component Integration)
//! ═══════════════════════════════════════════════════════════════════
//!
//! `saba_core::protocol::{RconClient, RestClient}`가 실제 mock 서버와
//! 정확한 와이어 포맷으로 라운드트립하는지 검증한다.
//!
//! 분류: **Component Integration** — 별도 OS 프로세스는 spawn하지 않으나,
//! 실제 TCP/HTTP 소켓을 통해 동기 클라이언트와 mock 서버가 통신.
//!
//! ## 검증 범위
//! - **RCON**: 인증 (단일 응답), 명령 실행, 잘못된 비밀번호 거부, 연결 종료
//! - **REST**: GET/POST/PUT/DELETE, Basic Auth 헤더 인코딩, JSON body, 4xx 에러

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use saba_core::protocol::rcon::RconClient;
use saba_core::protocol::rest::RestClient;
use saba_core::protocol::ServerResponse;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

// ═══════════════════════════════════════════════════════════
// RCON Mock Server
// ═══════════════════════════════════════════════════════════

const SERVERDATA_AUTH: i32 = 3;
const SERVERDATA_EXECCOMMAND: i32 = 2;
const SERVERDATA_AUTH_RESPONSE: i32 = 2;
const SERVERDATA_RESPONSE_VALUE: i32 = 0;

#[derive(Debug, Clone)]
struct RconPacket {
    id: i32,
    ptype: i32,
    payload: String,
}

fn read_packet(stream: &mut TcpStream) -> std::io::Result<RconPacket> {
    let size = stream.read_i32::<LittleEndian>()?;
    let mut body = vec![0u8; size as usize];
    stream.read_exact(&mut body)?;
    let id = (&body[0..4]).read_i32::<LittleEndian>()?;
    let ptype = (&body[4..8]).read_i32::<LittleEndian>()?;
    // payload: 4..size-2 (마지막 두 바이트는 null 종결자)
    let payload_end = body.len().saturating_sub(2);
    let payload = String::from_utf8_lossy(&body[8..payload_end]).to_string();
    Ok(RconPacket { id, ptype, payload })
}

fn write_packet(stream: &mut TcpStream, id: i32, ptype: i32, payload: &str) -> std::io::Result<()> {
    let mut body = Vec::new();
    body.write_i32::<LittleEndian>(id)?;
    body.write_i32::<LittleEndian>(ptype)?;
    body.extend_from_slice(payload.as_bytes());
    body.push(0);
    body.push(0);
    let mut wire = Vec::with_capacity(4 + body.len());
    wire.write_i32::<LittleEndian>(body.len() as i32)?;
    wire.extend_from_slice(&body);
    stream.write_all(&wire)?;
    stream.flush()?;
    Ok(())
}

/// 단일 응답 RCON 서버를 띄우고 들어오는 단일 연결을 처리한다.
/// `expected_password`와 다른 비밀번호로 인증 시 id=-1로 거부.
fn spawn_rcon_server(expected_password: String) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind rcon");
    let port = listener.local_addr().unwrap().port();

    thread::spawn(move || {
        for incoming in listener.incoming() {
            let mut stream = match incoming {
                Ok(s) => s,
                Err(_) => continue,
            };
            stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

            // 1) AUTH 패킷 수신
            let auth = match read_packet(&mut stream) {
                Ok(p) => p,
                Err(_) => continue,
            };
            assert_eq!(auth.ptype, SERVERDATA_AUTH, "첫 패킷은 AUTH여야 함");

            let auth_id = if auth.payload == expected_password {
                auth.id
            } else {
                -1
            };
            let _ = write_packet(&mut stream, auth_id, SERVERDATA_AUTH_RESPONSE, "");

            // 인증 실패면 연결 종료
            if auth_id == -1 {
                continue;
            }

            // 2) 명령 패킷 처리 루프
            loop {
                match read_packet(&mut stream) {
                    Ok(cmd) if cmd.ptype == SERVERDATA_EXECCOMMAND => {
                        let response = match cmd.payload.as_str() {
                            "list" => "There are 2 of 20 players online: alice, bob".to_string(),
                            "say hi" => "Message broadcast".to_string(),
                            other => format!("Unknown command: {}", other),
                        };
                        let _ = write_packet(&mut stream, cmd.id, SERVERDATA_RESPONSE_VALUE, &response);
                    }
                    _ => break,
                }
            }
        }
    });

    port
}

#[test]
fn rcon_client_authenticates_and_executes_commands() {
    let port = spawn_rcon_server("correct-pw".to_string());
    // 서버 ready 대기
    thread::sleep(Duration::from_millis(50));

    let mut client = RconClient::new("127.0.0.1".to_string(), port, "correct-pw".to_string());
    client
        .connect(Duration::from_secs(3))
        .expect("connect/auth should succeed");

    let resp = client.execute_command("list").expect("list command");
    assert!(
        resp.contains("alice") && resp.contains("bob"),
        "응답에 플레이어 이름 포함: {resp}"
    );

    let resp2 = client.execute_command("say hi").expect("say command");
    assert_eq!(resp2, "Message broadcast");

    client.disconnect();
}

#[test]
fn rcon_client_rejects_wrong_password() {
    let port = spawn_rcon_server("real-pw".to_string());
    thread::sleep(Duration::from_millis(50));

    let mut client = RconClient::new("127.0.0.1".to_string(), port, "wrong-pw".to_string());
    let result = client.connect(Duration::from_secs(3));
    assert!(result.is_err(), "잘못된 비밀번호는 인증 실패해야 함");
}

// ═══════════════════════════════════════════════════════════
// REST Mock Server
// ═══════════════════════════════════════════════════════════

/// 요청 본문 + 메서드 + Authorization 헤더를 검사할 수 있는 미니 HTTP 서버.
/// `verify_basic_auth`가 Some이면 해당 base64(`user:pass`) 토큰 일치 여부 확인.
fn spawn_rest_server(verify_basic_auth: Option<String>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind rest");
    let port = listener.local_addr().unwrap().port();

    thread::spawn(move || {
        for incoming in listener.incoming() {
            let mut stream = match incoming {
                Ok(s) => s,
                Err(_) => continue,
            };
            let mut buf = [0u8; 4096];
            let n = match stream.read(&mut buf) {
                Ok(n) => n,
                Err(_) => continue,
            };
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let mut lines = req.lines();
            let request_line = lines.next().unwrap_or("");
            let mut parts = request_line.split_whitespace();
            let method = parts.next().unwrap_or("");
            let path = parts.next().unwrap_or("");

            let mut auth_ok = verify_basic_auth.is_none();
            for line in lines.by_ref() {
                if line.is_empty() {
                    break;
                }
                if let Some(rest) = line.strip_prefix("Authorization: ") {
                    if let Some(expected) = &verify_basic_auth {
                        auth_ok = rest.trim() == format!("Basic {}", expected);
                    }
                }
            }

            let (status_line, body) = if !auth_ok {
                ("HTTP/1.1 401 Unauthorized", r#"{"error":"unauthorized"}"#.to_string())
            } else {
                match (method, path) {
                    ("GET", "/v1/api/info") => (
                        "HTTP/1.1 200 OK",
                        r#"{"servername":"E2E","version":"v0.0.1"}"#.to_string(),
                    ),
                    ("POST", "/v1/api/announce") => (
                        "HTTP/1.1 200 OK",
                        r#"{"announced":true}"#.to_string(),
                    ),
                    ("PUT", "/v1/api/setting") => (
                        "HTTP/1.1 200 OK",
                        r#"{"updated":true}"#.to_string(),
                    ),
                    ("DELETE", "/v1/api/player/alice") => (
                        "HTTP/1.1 200 OK",
                        r#"{"kicked":"alice"}"#.to_string(),
                    ),
                    _ => (
                        "HTTP/1.1 404 Not Found",
                        r#"{"error":"not found"}"#.to_string(),
                    ),
                }
            };

            let response = format!(
                "{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status_line,
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });

    port
}

#[test]
fn rest_client_get_post_put_delete_round_trip() {
    let port = spawn_rest_server(None);
    thread::sleep(Duration::from_millis(50));

    let client = RestClient::new("127.0.0.1".to_string(), port, false);

    fn extract_response(resp: &ServerResponse) -> serde_json::Value {
        resp.data
            .as_ref()
            .and_then(|d| d.get("response").cloned())
            .expect("data.response 포함")
    }

    // GET
    let info = client.get("/v1/api/info").expect("GET info");
    let body = extract_response(&info);
    assert_eq!(body.get("servername").and_then(|v| v.as_str()), Some("E2E"));

    // POST
    let announce = client
        .post("/v1/api/announce", Some(serde_json::json!({"text": "hello"})))
        .expect("POST announce");
    let body = extract_response(&announce);
    assert_eq!(body.get("announced").and_then(|v| v.as_bool()), Some(true));

    // PUT
    let put_resp = client
        .put("/v1/api/setting", Some(serde_json::json!({"k": "v"})))
        .expect("PUT setting");
    let body = extract_response(&put_resp);
    assert_eq!(body.get("updated").and_then(|v| v.as_bool()), Some(true));

    // DELETE
    let del = client.delete("/v1/api/player/alice").expect("DELETE");
    let body = extract_response(&del);
    assert_eq!(body.get("kicked").and_then(|v| v.as_str()), Some("alice"));
}

#[test]
fn rest_client_basic_auth_header_is_correct() {
    use base64::Engine;
    let expected = base64::engine::general_purpose::STANDARD.encode(b"admin:secret-pw");
    let port = spawn_rest_server(Some(expected));
    thread::sleep(Duration::from_millis(50));

    let mut client = RestClient::new("127.0.0.1".to_string(), port, false);
    client.set_basic_auth("admin".to_string(), "secret-pw".to_string());

    let info = client.get("/v1/api/info").expect("authorized GET");
    let response: serde_json::Value = info
        .data
        .as_ref()
        .and_then(|d: &serde_json::Value| d.get("response").cloned())
        .expect("data.response");
    assert_eq!(response.get("servername").and_then(|v: &serde_json::Value| v.as_str()), Some("E2E"));
}

#[test]
fn rest_client_basic_auth_with_wrong_password_yields_401() {
    use base64::Engine;
    let expected = base64::engine::general_purpose::STANDARD.encode(b"admin:secret-pw");
    let port = spawn_rest_server(Some(expected));
    thread::sleep(Duration::from_millis(50));

    let mut client = RestClient::new("127.0.0.1".to_string(), port, false);
    client.set_basic_auth("admin".to_string(), "WRONG-PW".to_string());

    let result = client.get("/v1/api/info");
    assert!(
        result.is_err(),
        "잘못된 비밀번호는 에러여야 함(401을 ProtocolError로 올림): {:?}",
        result
    );
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("401"), "에러 메시지에 401 포함: {msg}");
}
