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

        tracing::info!("RCON client connected to {}:{}", self.host, self.port);
        Ok(())
    }

    /// 인증 패킷 전송
    fn authenticate(&mut self) -> Result<(), ProtocolError> {
        self.request_id = 1;
        let password = self.password.clone();
        let response_id = self.send_command_internal(self.request_id, 3, &password)?;

        if response_id == -1 {
            return Err(ProtocolError::AuthError("Authentication failed: invalid password".to_string()));
        }

        Ok(())
    }

    /// 명령어 실행
    pub fn execute_command(&mut self, command: &str) -> Result<String, ProtocolError> {
        self.request_id += 1;
        self.send_command_internal(self.request_id, 2, command)?;
        Ok(format!("Command executed: {}", command))
    }

    /// 내부 명령어 전송 함수
    /// command_type: 2 = command, 3 = auth
    fn send_command_internal(&mut self, request_id: u32, command_type: i32, payload: &str) -> Result<i32, ProtocolError> {
        {
            let stream = self.stream.as_mut()
                .ok_or_else(|| ProtocolError::ConnectionError("Not connected".to_string()))?;

            // 패킷 생성: [ID][타입][페이로드][패딩]
            let mut packet = Vec::new();
            packet.write_i32::<LittleEndian>(request_id as i32)
                .map_err(|e| ProtocolError::ProtocolError(format!("Failed to write request ID: {}", e)))?;
            packet.write_i32::<LittleEndian>(command_type)
                .map_err(|e| ProtocolError::ProtocolError(format!("Failed to write command type: {}", e)))?;
            packet.extend_from_slice(payload.as_bytes());
            packet.extend_from_slice(&[0, 0]); // 패딩

            // 패킷 크기
            let packet_size = packet.len() as u32;

            // 전송: [크기][패킷]
            stream.write_u32::<LittleEndian>(packet_size)
                .map_err(|e| ProtocolError::ProtocolError(format!("Failed to write packet size: {}", e)))?;
            stream.write_all(&packet)
                .map_err(|e| ProtocolError::ProtocolError(format!("Failed to send packet: {}", e)))?;
        }

        // 응답 수신
        self.read_response()
    }

    /// 응답 읽기
    fn read_response(&mut self) -> Result<i32, ProtocolError> {
        let stream = self.stream.as_mut()
            .ok_or_else(|| ProtocolError::ConnectionError("Not connected".to_string()))?;

        // 패킷 크기 읽기
        let packet_size = stream.read_u32::<LittleEndian>()
            .map_err(|e| ProtocolError::ProtocolError(format!("Failed to read packet size: {}", e)))? as usize;

        if packet_size > 4096 {
            return Err(ProtocolError::ProtocolError(format!("Packet size too large: {}", packet_size)));
        }

        // 패킷 데이터 읽기
        let mut packet = vec![0u8; packet_size];
        stream.read_exact(&mut packet)
            .map_err(|e| ProtocolError::ProtocolError(format!("Failed to read packet data: {}", e)))?;

        // Request ID 읽기
        let mut cursor = &packet[..];
        let request_id = cursor.read_i32::<LittleEndian>()
            .map_err(|e| ProtocolError::ProtocolError(format!("Failed to parse request ID: {}", e)))?;

        Ok(request_id)
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
