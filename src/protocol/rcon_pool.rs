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
                        Err(e) => {
                            // 명령 실행 중 에러 → 연결 제거하고 아래에서 재연결 시도
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

        let response = client.execute_command(command)?;
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

    /// 모든 RCON 연결 해제
    #[allow(dead_code)]
    pub fn clear(&self) {
        let mut conns = self.connections.lock().unwrap();
        let count = conns.len();
        conns.clear();
        if count > 0 {
            tracing::info!("RCON pool: cleared {} connections", count);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_creation() {
        let pool = RconPool::new();
        // 빈 풀에서 remove는 에러 없이 동작
        pool.remove("nonexistent");
    }

    #[test]
    fn test_pool_clear() {
        let pool = RconPool::new();
        pool.clear();
        // 빈 풀 clear는 정상 동작
    }
}
