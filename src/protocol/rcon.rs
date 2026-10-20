//! RCON(Source RCON 프로토콜) 클라이언트 — 명령 경로의 마지막 단계
//!
//! 명령은 STDIN → REST → RCON 순서로 보낸다. RCON은 module.toml `[protocols] supported`에 `rcon`만 있고
//! `stdin`·`rest`가 없는 모듈(정말 RCON만 받는 서버)에만 쓴다 (`ModuleMetadata::rcon_only_port`).
//! 접속은 늘 127.0.0.1이다. 비밀번호는 데몬이 인스턴스마다 만들어 인스턴스 폴더의 `rcon.json`(주인만 읽기)에
//! 두고, 모듈 config의 `rcon_port`·`rcon_password`로 넘겨 모듈이 게임 설정에 쓰게 한다.
//! 비밀번호는 로그·오류·API 응답에 넣지 않는다.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// 인스턴스 폴더 안의 접속 정보 파일
pub const ACCESS_FILE: &str = "rcon.json";

const AUTH: i32 = 3;
const AUTH_RESPONSE: i32 = 2;
const EXEC: i32 = 2;
const RESPONSE_VALUE: i32 = 0;
/// 보내는 명령 본문 상한 (Source 규격의 패킷 본문 상한)
const MAX_COMMAND: usize = 4096;
/// 받는 패킷 하나의 크기 상한과 답 전체의 상한
const MAX_PACKET: usize = 64 * 1024;
const MAX_REPLY: usize = 1024 * 1024;
/// 접속·인증·답 전체의 시간 상한
const TIMEOUT: Duration = Duration::from_secs(5);

/// 데몬이 접속할 포트와 비밀번호
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Access {
    pub port: u16,
    pub password: String,
}

impl std::fmt::Debug for Access {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Access").field("port", &self.port).field("password", &"***").finish()
    }
}

/// 저장된 접속 정보. 없거나 읽지 못하면 None
pub fn load(instance_dir: &Path) -> Option<Access> {
    let text = std::fs::read_to_string(instance_dir.join(ACCESS_FILE)).ok()?;
    serde_json::from_str::<Access>(&text).ok().filter(|a| a.port > 0 && !a.password.is_empty())
}

/// 접속 정보를 돌려준다. 없으면 `default_port`와 새 비밀번호로 만들어 저장한다 (주인만 읽기, 원자적 쓰기)
pub fn ensure(instance_dir: &Path, default_port: u16) -> anyhow::Result<Access> {
    if let Some(access) = load(instance_dir) {
        return Ok(access);
    }
    let access = Access { port: default_port, password: crate::instance::generate_random_password() };
    std::fs::create_dir_all(instance_dir)?;
    let path = instance_dir.join(ACCESS_FILE);
    let tmp = instance_dir.join(format!("{}.tmp", ACCESS_FILE));
    {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(serde_json::to_string_pretty(&access)?.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(access)
}

/// RCON 실패. 메시지에 비밀번호·서버 답을 넣지 않는다
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RconError {
    #[error("the command is empty, too long or spans several lines")]
    InvalidCommand,
    #[error("could not connect to RCON on 127.0.0.1:{0}")]
    Connect(u16),
    #[error("the server rejected the RCON password")]
    AuthFailed,
    #[error("the server did not answer over RCON in time")]
    Timeout,
    #[error("the server sent a malformed RCON reply")]
    Protocol,
}

impl RconError {
    pub fn code(&self) -> &'static str {
        match self {
            RconError::InvalidCommand => "invalid_argument",
            RconError::Connect(_) => "rcon_unreachable",
            RconError::AuthFailed => "rcon_auth_failed",
            RconError::Timeout => "rcon_timeout",
            RconError::Protocol => "rcon_failed",
        }
    }
}

/// 127.0.0.1:`port`에 인증하고 명령 한 줄을 실행해 답을 돌려준다
pub async fn exec(port: u16, password: &str, command: &str) -> Result<String, RconError> {
    if command.trim().is_empty() || command.len() > MAX_COMMAND || command.contains(['\r', '\n', '\0']) {
        return Err(RconError::InvalidCommand);
    }
    match tokio::time::timeout(TIMEOUT, session(port, password, command)).await {
        Ok(result) => result,
        Err(_) => Err(RconError::Timeout),
    }
}

async fn session(port: u16, password: &str, command: &str) -> Result<String, RconError> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.map_err(|_| RconError::Connect(port))?;

    write_packet(&mut stream, 1, AUTH, password.as_bytes()).await?;
    // Source 서버는 인증 답 앞에 빈 RESPONSE_VALUE를 하나 보낸다
    loop {
        let (id, kind, _) = read_packet(&mut stream).await?;
        if kind == AUTH_RESPONSE {
            if id == -1 {
                return Err(RconError::AuthFailed);
            }
            break;
        }
    }

    // 명령 뒤에 빈 RESPONSE_VALUE를 보내면 서버가 명령 답을 다 보낸 뒤 그 id로 답한다: 여러 패킷 답의 끝 표시
    write_packet(&mut stream, 2, EXEC, command.as_bytes()).await?;
    write_packet(&mut stream, 3, RESPONSE_VALUE, b"").await?;
    let mut reply = Vec::new();
    loop {
        let (id, _, body) = read_packet(&mut stream).await?;
        match id {
            2 => {
                if reply.len() + body.len() > MAX_REPLY {
                    return Err(RconError::Protocol);
                }
                reply.extend_from_slice(&body);
            }
            3 => break,
            _ => {}
        }
    }
    // 조각마다가 아니라 모은 뒤 한 번에 바꾼다 (패킷 경계에서 글자가 잘리지 않게)
    Ok(String::from_utf8_lossy(&reply).into_owned())
}

async fn write_packet(stream: &mut TcpStream, id: i32, kind: i32, body: &[u8]) -> Result<(), RconError> {
    let mut packet = Vec::with_capacity(body.len() + 14);
    packet.extend_from_slice(&((body.len() + 10) as i32).to_le_bytes());
    packet.extend_from_slice(&id.to_le_bytes());
    packet.extend_from_slice(&kind.to_le_bytes());
    packet.extend_from_slice(body);
    packet.extend_from_slice(&[0, 0]);
    stream.write_all(&packet).await.map_err(|_| RconError::Protocol)
}

async fn read_packet(stream: &mut TcpStream) -> Result<(i32, i32, Vec<u8>), RconError> {
    let mut size = [0u8; 4];
    stream.read_exact(&mut size).await.map_err(|_| RconError::Protocol)?;
    let size = i32::from_le_bytes(size);
    if !(10..=MAX_PACKET as i32).contains(&size) {
        return Err(RconError::Protocol);
    }
    let mut rest = vec![0u8; size as usize];
    stream.read_exact(&mut rest).await.map_err(|_| RconError::Protocol)?;
    let id = i32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
    let kind = i32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]);
    let mut body = rest[8..].to_vec();
    // 끝의 NUL 두 개 (빠뜨리는 서버도 있어 있는 만큼만 뗀다)
    while body.last() == Some(&0) {
        body.pop();
    }
    Ok((id, kind, body))
}

/// 시험용 RCON 서버: `password`로 인증하고, 받은 명령을 기록해 `reply(명령)`을 답한다.
/// `source_style`이면 인증 답 앞에 빈 RESPONSE_VALUE를 보내고, 긴 답을 두 패킷으로 나눈다
#[cfg(test)]
pub(crate) async fn fake_server(
    password: &str,
    source_style: bool,
) -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    (port, serve(listener, password, source_style))
}

/// `fake_server`를 정해 둔 포트에 띄운다
#[cfg(test)]
pub(crate) async fn fake_server_on(port: u16, password: &str) -> std::sync::Arc<std::sync::Mutex<Vec<String>>> {
    serve(tokio::net::TcpListener::bind(("127.0.0.1", port)).await.unwrap(), password, false)
}

#[cfg(test)]
fn serve(listener: tokio::net::TcpListener, password: &str, source_style: bool) -> std::sync::Arc<std::sync::Mutex<Vec<String>>> {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let (password, log) = (password.to_string(), seen.clone());
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let (password, log) = (password.clone(), log.clone());
            tokio::spawn(async move {
                while let Ok((id, kind, body)) = read_packet(&mut stream).await {
                    let text = String::from_utf8_lossy(&body).to_string();
                    match kind {
                        AUTH => {
                            if source_style {
                                let _ = write_packet(&mut stream, id, RESPONSE_VALUE, b"").await;
                            }
                            let ok = text == password;
                            let _ = write_packet(&mut stream, if ok { id } else { -1 }, AUTH_RESPONSE, b"").await;
                            if !ok {
                                return;
                            }
                        }
                        EXEC => {
                            log.lock().unwrap().push(text.clone());
                            let answer = format!("ran: {}", text).into_bytes();
                            let (a, b) = if source_style { answer.split_at(answer.len() / 2) } else { (&answer[..], &[][..]) };
                            let _ = write_packet(&mut stream, id, RESPONSE_VALUE, a).await;
                            if !b.is_empty() {
                                let _ = write_packet(&mut stream, id, RESPONSE_VALUE, b).await;
                            }
                        }
                        _ => {
                            let _ = write_packet(&mut stream, id, RESPONSE_VALUE, b"").await;
                        }
                    }
                }
            });
        }
    });
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_command_runs_after_authenticating_and_returns_the_whole_reply() {
        for source_style in [false, true] {
            let (port, seen) = fake_server("pw-123456", source_style).await;
            let reply = exec(port, "pw-123456", "list 한글").await.unwrap();
            assert_eq!(reply, "ran: list 한글", "source_style={source_style}");
            assert_eq!(seen.lock().unwrap().as_slice(), ["list 한글"]);
        }
    }

    #[tokio::test]
    async fn a_wrong_password_or_a_closed_port_fails_without_naming_the_password() {
        let (port, seen) = fake_server("right-password", false).await;
        let err = exec(port, "wrong-password", "list").await.unwrap_err();
        assert_eq!(err, RconError::AuthFailed);
        assert!(!err.to_string().contains("password\""), "{err}");
        assert!(seen.lock().unwrap().is_empty(), "nothing runs before authenticating");

        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert_eq!(exec(free, "x", "list").await.unwrap_err().code(), "rcon_unreachable");
        assert_eq!(exec(port, "right-password", "say a\nop me").await.unwrap_err(), RconError::InvalidCommand);
    }

    #[test]
    fn the_access_file_is_made_once_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).is_none());
        let first = ensure(dir.path(), 27015).unwrap();
        assert_eq!(first.port, 27015);
        assert_eq!(first.password.len(), 16);
        assert_eq!(ensure(dir.path(), 1).unwrap(), first, "an existing file wins over the module default");
        assert!(!format!("{first:?}").contains(&first.password));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(ACCESS_FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "only the owner can read it");
        }
    }
}
