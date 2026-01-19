/**
 * 통합 통신 클라이언트 아키텍처 설계
 * 
 * ## 목표
 * - RCON과 REST API를 추상화한 범용 클라이언트
 * - 게임별 프로토콜 다름을 숨김
 * - 모듈(Python)이 간단한 HTTP 호출로 서버 제어
 * 
 * ## 아키텍처
 * 
 * ```
 * Module (Python)
 *   └─ POST /api/instance/:id/command
 *        {"type": "rcon", "command": "say hello"}
 *        {"type": "rest", "endpoint": "/api/info", "method": "GET"}
 *
 * Daemon (Rust) 
 *   ├─ src/protocol/mod.rs
 *   ├─ src/protocol/rcon.rs        (RCON 클라이언트)
 *   ├─ src/protocol/rest.rs        (REST API 클라이언트)
 *   ├─ src/protocol/client.rs      (통합 클라이언트)
 *   └─ src/ipc/mod.rs              (API 엔드포인트)
 *
 * ServerInstance
 *   ├─ type: "rcon" | "rest" | "both"
 *   ├─ rcon_host: "127.0.0.1"
 *   ├─ rcon_port: 25575
 *   ├─ rcon_password: "password"
 *   ├─ rest_host: "127.0.0.1"
 *   ├─ rest_port: 8212
 *   ├─ rest_username: "admin"
 *   └─ rest_password: "password"
 * ```
 * 
 * ## 통신 타입 정의
 * 
 * ### RCON (Minecraft, 일부 Palworld)
 * - 프로토콜: RCON (TCP, 부호화)
 * - 명령어: "say hello", "stop", "save-all"
 * - 응답: 텍스트
 * 
 * ### REST API (Palworld)
 * - 프로토콜: HTTP/HTTPS
 * - Basic Auth 또는 키 기반 인증
 * - 엔드포인트: GET/POST /api/v1/server, /api/v1/players, etc.
 * 
 * ## 구현 계획
 * 
 * ### 1단계: 기본 구조 (이번 작업)
 * - [ ] src/protocol/mod.rs: 모듈 정의
 * - [ ] src/protocol/rcon.rs: RCON 클라이언트
 * - [ ] src/protocol/rest.rs: REST API 클라이언트
 * - [ ] src/protocol/client.rs: 통합 클라이언트
 * 
 * ### 2단계: API 엔드포인트
 * - [ ] POST /api/instance/:id/command: 명령어 실행
 * - [ ] GET  /api/instance/:id/status: 서버 상태 조회
 * - [ ] POST /api/instance/:id/control: 서버 제어 (start/stop/restart)
 * 
 * ### 3단계: 모듈 통합
 * - [ ] lifecycle.py 간소화 (HTTP 호출로 변경)
 * - [ ] Palworld 모듈 업데이트
 * - [ ] Minecraft 모듈 업데이트
 * 
 * ## 데이터 구조
 * 
 * ### CommandRequest
 * ```json
 * {
 *   "type": "rcon" | "rest",
 *   "command": "say hello",        // RCON의 경우
 *   "endpoint": "/api/info",       // REST의 경우
 *   "method": "GET" | "POST",
 *   "body": {...}
 * }
 * ```
 * 
 * ### CommandResponse
 * ```json
 * {
 *   "success": true,
 *   "data": "...",
 *   "error": null
 * }
 * ```
 * 
 * ## 에러 처리
 * - 연결 실패: ConnectionError
 * - 인증 실패: AuthError
 * - 타임아웃: TimeoutError
 * - 명령어 오류: CommandError
 * - 프로토콜 오류: ProtocolError
 */
