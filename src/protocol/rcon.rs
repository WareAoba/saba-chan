use super::ProtocolError;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

/// RCON 클라이언트 (Minecraft, Palworld 호환)
/// 
/// RCON 프로토콜 명세:
/// - TCP 기반 통신
/// - 모든 패킷은 4바이트 패킷 크기 + 데이터로 구성
/// - 인증 후 명령어 송수신
#[derive(Debug)]
pub struct RconClient {
    stream: Option<TcpStream>,
    host: String,
    port: u16,
    password: String,
    request_id: u32,
}

impl RconClient {
    pub fn new(host: String, port: u16, password: String) -> Self {
        Self {
            stream: None,
            host,
            port,
            password,
            request_id: 0,
        }
    }

    /// RCON 서버에 연결하고 인증
    pub fn connect(&mut self, timeout: Duration) -> Result<(), ProtocolError> {
        let addr = format!("{}:{}", self.host, self.port);
        
        let stream = TcpStream::connect(&addr)
            .map_err(|e| ProtocolError::ConnectionError(format!("Failed to connect to {}: {}", addr, e)))?;

        stream.set_read_timeout(Some(timeout))
            .map_err(|e| ProtocolError::ConnectionError(format!("Failed to set read timeout: {}", e)))?;
        
        stream.set_write_timeout(Some(timeout))
            .map_err(|e| ProtocolError::ConnectionError(format!("Failed to set write timeout: {}", e)))?;

        self.stream = Some(stream);

        // 인증
        self.authenticate()?;

        // 인증 과정에서 짧은 타임아웃으로 변경되었으므로, 명령어 실행용으로 원래 타임아웃 복원
        if let Some(stream) = &self.stream {
            let _ = stream.set_read_timeout(Some(timeout));
        }

        tracing::info!("RCON client connected to {}:{}", self.host, self.port);
        Ok(())
    }

    /// 인증 패킷 전송
    /// RCON 서버에 따라 인증 응답이 1개 또는 2개:
    ///   - 일부 서버: AUTH_RESPONSE(type 2) 1개만 전송
    ///   - 일부 서버: RESPONSE_VALUE(type 0) + AUTH_RESPONSE(type 2) 2개 전송
    fn authenticate(&mut self) -> Result<(), ProtocolError> {
        self.request_id = 1;
        let password = self.password.clone();
        let (first_id, _payload) = self.send_command_internal(self.request_id, 3, &password)?;

        // 첫 번째 응답이 바로 -1이면 인증 실패 (싱글 응답 서버)
        if first_id == -1 {
            return Err(ProtocolError::AuthError("Authentication failed: invalid password".to_string()));
        }

        // 일부 서버(Minecraft 등)는 두 번째 응답을 추가로 보냄
        // 짧은 타임아웃(500ms)으로 두 번째 응답을 시도 — 없으면 싱글 응답 서버로 판단
        if let Some(stream) = &self.stream {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        }

        match self.read_response() {
            Ok((second_id, _)) => {
                if second_id == -1 {
                    return Err(ProtocolError::AuthError("Authentication failed: invalid password".to_string()));
                }
                tracing::debug!("Dual auth response server — both responses OK");
            }
            Err(ref e) => {
                // 타임아웃(WouldBlock/TimedOut)은 정상 — 단일 응답 서버
                // 연결 종료(UnexpectedEof)는 서버가 연결을 끊은 것 — 인증 실패 가능
                match e {
                    ProtocolError::ConnectionError(_) => {
                        tracing::warn!(
                            "RCON server closed connection after first auth response (possible auth failure or server issue)"
                        );
                        return Err(ProtocolError::AuthError(
                            "Server closed connection after auth response — check RCON password and server.properties (enable-rcon, rcon.password)".to_string()
                        ));
                    }
                    ProtocolError::TimeoutError(_) => {
                        tracing::debug!("No second auth response (single-response server)");
                    }
                    _ => {
                        tracing::debug!("No second auth response (error: {})", e);
                    }
                }
            }
        }

        // 인증 후 연결 상태 확인 — peek으로 소켓이 살아있는지 검증
        if let Some(stream) = &self.stream {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
            let mut peek_buf = [0u8; 1];
            match stream.peek(&mut peek_buf) {
                Ok(0) => {
                    // 서버가 이미 연결을 닫음
                    return Err(ProtocolError::AuthError(
                        "Server closed connection after authentication — verify rcon.password in server.properties".to_string()
                    ));
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {
                    // 데이터 없이 타임아웃 — 정상 (서버가 아직 데이터를 보내지 않은 상태)
                }
                _ => {
                    // 데이터 있거나 기타 — 정상
                }
            }
        }

        Ok(())
    }

    /// 명령어 실행 — 실제 응답 페이로드를 파싱하여 반환
    pub fn execute_command(&mut self, command: &str) -> Result<String, ProtocolError> {
        self.request_id += 1;
        let (_id, payload) = self.send_command_internal(self.request_id, 2, command)?;
        Ok(payload)
    }

    /// 내부 명령어 전송 함수
    /// command_type: 2 = command, 3 = auth
    /// Returns (response_id, payload_string)
    fn send_command_internal(&mut self, request_id: u32, command_type: i32, payload: &str) -> Result<(i32, String), ProtocolError> {
        {
            let stream = self.stream.as_mut()
                .ok_or_else(|| ProtocolError::ConnectionError("Not connected".to_string()))?;

            // 패킷 본문: [ID (4)][타입 (4)][페이로드][null (1)][null (1)]
            let mut body = Vec::new();
            body.write_i32::<LittleEndian>(request_id as i32)
                .map_err(|e| ProtocolError::Protocol(format!("Failed to write request ID: {}", e)))?;
            body.write_i32::<LittleEndian>(command_type)
                .map_err(|e| ProtocolError::Protocol(format!("Failed to write command type: {}", e)))?;
            body.extend_from_slice(payload.as_bytes());
            body.extend_from_slice(&[0, 0]); // null terminator + empty string

            // 전송 버퍼: [크기 (4)][본문] — 단일 write로 전송
            // Minecraft RCON 서버는 recv() 한 번으로 전체 패킷을 읽으므로,
            // 분할 전송 시 서버가 불완전한 패킷으로 판단하여 연결을 끊을 수 있음
            let mut wire = Vec::with_capacity(4 + body.len());
            wire.write_i32::<LittleEndian>(body.len() as i32)
                .map_err(|e| ProtocolError::Protocol(format!("Failed to write packet size: {}", e)))?;
            wire.extend_from_slice(&body);

            stream.write_all(&wire)
                .map_err(|e| ProtocolError::Protocol(format!("Failed to send packet: {}", e)))?;
            stream.flush()
                .map_err(|e| ProtocolError::Protocol(format!("Failed to flush stream: {}", e)))?;
        }

        // 응답 수신
        self.read_response()
    }

    /// 응답 읽기 — (request_id, payload_string) 반환
    fn read_response(&mut self) -> Result<(i32, String), ProtocolError> {
        let stream = self.stream.as_mut()
            .ok_or_else(|| ProtocolError::ConnectionError("Not connected".to_string()))?;

        // 패킷 크기 읽기 — 에러 종류를 구분하여 적절한 ProtocolError 반환
        let packet_size = stream.read_u32::<LittleEndian>()
            .map_err(|e| {
                match e.kind() {
                    std::io::ErrorKind::UnexpectedEof => {
                        ProtocolError::ConnectionError(format!(
                            "Server closed connection (failed to read packet size: {})", e
                        ))
                    }
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
                        ProtocolError::TimeoutError(format!(
                            "Read timed out waiting for response: {}", e
                        ))
                    }
                    _ => {
                        ProtocolError::Protocol(format!("Failed to read packet size: {}", e))
                    }
                }
            })? as usize;

        if packet_size > 4096 {
            return Err(ProtocolError::Protocol(format!("Packet size too large: {}", packet_size)));
        }

        // 패킷 데이터 읽기
        let mut packet = vec![0u8; packet_size];
        stream.read_exact(&mut packet)
            .map_err(|e| ProtocolError::Protocol(format!("Failed to read packet data: {}", e)))?;

        // 패킷 구조: [4 bytes request_id][4 bytes type][payload...][2 bytes padding]
        if packet.len() < 10 {
            // 최소 4(id) + 4(type) + 2(padding) = 10 bytes
            return Err(ProtocolError::Protocol("Response packet too small".to_string()));
        }

        let mut cursor = &packet[..];
        let request_id = cursor.read_i32::<LittleEndian>()
            .map_err(|e| ProtocolError::Protocol(format!("Failed to parse request ID: {}", e)))?;

        // Skip response type (4 bytes)
        let _response_type = cursor.read_i32::<LittleEndian>()
            .map_err(|e| ProtocolError::Protocol(format!("Failed to parse response type: {}", e)))?;

        // Remaining bytes minus 2-byte null padding = payload
        let payload_bytes = &packet[8..packet.len().saturating_sub(2)];
        let payload = String::from_utf8_lossy(payload_bytes)
            .trim_end_matches('\0')
            .to_string();

        Ok((request_id, payload))
    }

    /// 연결이 살아있는지 확인
    pub fn is_connected(&self) -> bool {
        match &self.stream {
            None => false,
            Some(stream) => {
                // peek으로 소켓 상태 확인 (짧은 타임아웃)
                let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
                let mut peek_buf = [0u8; 1];
                match stream.peek(&mut peek_buf) {
                    Ok(0) => false, // 서버가 연결을 닫음
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => true, // 데이터 없이 타임아웃 = 정상
                    Err(_) => false, // 기타 에러 = 연결 끊김
                    Ok(_) => true,   // 데이터 있음 = 연결 살아있음
                }
            }
        }
    }

    /// RCON 설정(host/port/password)이 동일한지 확인
    pub fn matches(&self, host: &str, port: u16, password: &str) -> bool {
        self.host == host && self.port == port && self.password == password
    }

    /// 연결 해제
    pub fn disconnect(&mut self) {
        self.stream = None;
        tracing::info!("RCON client disconnected from {}:{}", self.host, self.port);
    }
}

impl Drop for RconClient {
    fn drop(&mut self) {
        self.disconnect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rcon_client_creation() {
        let client = RconClient::new("127.0.0.1".to_string(), 25575, "password".to_string());
        assert_eq!(client.host, "127.0.0.1");
        assert_eq!(client.port, 25575);
        assert_eq!(client.password, "password");
    }

    #[test]
    fn test_rcon_client_not_connected() {
        let mut client = RconClient::new("127.0.0.1".to_string(), 25575, "password".to_string());
        let result = client.execute_command("say hello");
        assert!(result.is_err());
    }
}
