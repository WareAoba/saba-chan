use super::rcon::RconClient;
use super::ProtocolError;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/// 인스턴스별 RCON 연결을 유지/재사용하는 풀
///
/// 매 명령어마다 TCP 연결 + 인증을 반복하지 않고,
/// 서버가 실행 중인 동안 연결을 유지하여 재사용한다.
///
/// 동기 Mutex 사용: RconClient 내부의 TcpStream이 !Send + !Sync 이슈 없이
/// 동기적으로 동작하므로, tokio::Mutex 대신 std::sync::Mutex 사용.
/// lock은 매우 짧게 잡으므로 async 컨텍스트에서도 안전.
#[derive(Debug)]
pub struct RconPool {
    /// instance_id → RconClient 매핑
    connections: Mutex<HashMap<String, RconClient>>,
}

impl Default for RconPool {
    fn default() -> Self {
        Self::new()
    }
}

impl RconPool {
    pub fn new() -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
        }
    }

    /// 인스턴스용 RCON 연결을 가져오거나, 없으면/끊겼으면 새로 생성하여 명령어 실행
    ///
    /// 내부적으로:
    /// 1. 기존 연결이 있고 살아있으면 → 그대로 사용
    /// 2. 기존 연결이 있지만 설정이 변경됐거나 끊겼으면 → 재연결
    /// 3. 연결이 없으면 → 새로 연결
    /// 4. 명령어 실행 중 에러 발생 → 연결 제거 후 1회 재시도
    pub fn execute(
        &self,
        instance_id: &str,
        host: &str,
        port: u16,
        password: &str,
        command: &str,
        timeout: Duration,
    ) -> Result<String, ProtocolError> {
        // 1단계: 기존 연결이 유효한지 확인하고, 유효하면 명령어 실행
        {
            let mut conns = self.connections.lock().unwrap();
            if let Some(client) = conns.get_mut(instance_id) {
                // 설정이 변경됐으면 기존 연결 제거
                if !client.matches(host, port, password) {
                    tracing::info!(
                        "RCON pool: config changed for '{}', reconnecting",
                        instance_id
                    );
                    conns.remove(instance_id);
                } else if client.is_connected() {
                    // 기존 연결 사용하여 명령어 실행
                    match client.execute_command(command) {
                        Ok(response) => {
                            tracing::debug!(
                                "RCON pool: reused connection for '{}', command='{}'",
                                instance_id,
                                command
                            );
                            return Ok(response);
                        }
                        Err(e) if client.delivered() => {
                            // 명령은 이미 갔다: 다시 보내면 두 번 실행된다 (save-all 두 번 등)
                            conns.remove(instance_id);
                            return Err(ProtocolError::CommandError(format!("sent, but no reply: {}", e)));
                        }
                        Err(e) => {
                            // 보내기 전에 끊긴 연결 → 연결 제거하고 아래에서 재연결 시도
                            tracing::warn!(
                                "RCON pool: command failed on existing connection for '{}': {}, will reconnect",
                                instance_id,
                                e
                            );
                            conns.remove(instance_id);
                        }
                    }
                } else {
                    // 연결이 끊겨 있으면 제거
                    tracing::info!(
                        "RCON pool: stale connection for '{}', reconnecting",
                        instance_id
                    );
                    conns.remove(instance_id);
                }
            }
        }

        // 2단계: 새 연결 생성 + 인증 + 명령어 실행
        let mut client = RconClient::new(host.to_string(), port, password.to_string());
        client.connect(timeout)?;

        let response = client.execute_command(command).map_err(|e| {
            if client.delivered() {
                ProtocolError::CommandError(format!("sent, but no reply: {}", e))
            } else {
                e
            }
        })?;
        tracing::info!(
            "RCON pool: new connection established for '{}', command='{}'",
            instance_id,
            command
        );

        // 연결 저장 (재사용을 위해)
        {
            let mut conns = self.connections.lock().unwrap();
            conns.insert(instance_id.to_string(), client);
        }

        Ok(response)
    }

    /// 특정 인스턴스의 RCON 연결을 명시적으로 제거
    /// (서버 정지 시 호출)
    pub fn remove(&self, instance_id: &str) {
        let mut conns = self.connections.lock().unwrap();
        if conns.remove(instance_id).is_some() {
            tracing::info!("RCON pool: removed connection for '{}'", instance_id);
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    /// 명령이 서버까지 갔는데 답이 없으면 다시 보내지 않는다 (save-all 두 번 실행 회귀)
    #[test]
    fn delivered_command_is_not_resent_on_timeout() {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let commands = Arc::new(AtomicUsize::new(0));
        let seen = commands.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut s = stream.unwrap();
                let mut read_packet = |s: &mut std::net::TcpStream| -> Option<(i32, i32)> {
                    let mut len = [0u8; 4];
                    s.read_exact(&mut len).ok()?;
                    let mut body = vec![0u8; i32::from_le_bytes(len) as usize];
                    s.read_exact(&mut body).ok()?;
                    Some((i32::from_le_bytes(body[0..4].try_into().unwrap()), i32::from_le_bytes(body[4..8].try_into().unwrap())))
                };
                let Some((id, _)) = read_packet(&mut s) else { continue };
                let mut reply = Vec::new();
                reply.extend_from_slice(&10i32.to_le_bytes());
                reply.extend_from_slice(&id.to_le_bytes());
                reply.extend_from_slice(&2i32.to_le_bytes());
                reply.extend_from_slice(&[0, 0]);
                s.write_all(&reply).unwrap();
                // 첫 명령에만 답한다 (연결이 풀에 남게). 그 뒤 명령은 받기만 한다
                while let Some((id, kind)) = read_packet(&mut s) {
                    if kind == 2 && seen.fetch_add(1, Ordering::SeqCst) == 0 {
                        let mut ok = Vec::new();
                        ok.extend_from_slice(&10i32.to_le_bytes());
                        ok.extend_from_slice(&id.to_le_bytes());
                        ok.extend_from_slice(&0i32.to_le_bytes());
                        ok.extend_from_slice(&[0, 0]);
                        s.write_all(&ok).unwrap();
                    }
                }
            }
        });
        let pool = RconPool::new();
        pool.execute("i", "127.0.0.1", port, "pw", "list", Duration::from_millis(300)).unwrap();
        let err = pool.execute("i", "127.0.0.1", port, "pw", "save-all", Duration::from_millis(300)).unwrap_err();
        assert!(matches!(err, ProtocolError::CommandError(_)), "{:?}", err);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(commands.load(Ordering::SeqCst), 2, "save-all must reach the server once");
    }

    #[test]
    fn test_pool_creation() {
        let pool = RconPool::new();
        // 빈 풀에서 remove는 에러 없이 동작
        pool.remove("nonexistent");
    }

}
