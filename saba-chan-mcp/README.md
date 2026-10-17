# saba-chan-mcp

saba-chan 데몬을 **AI 에이전트가 자연어로 제어**하도록 하는 MCP(Model Context Protocol) 래퍼.

어떤 MCP 호환 에이전트(Claude Desktop, Cursor, VS Code Copilot 등)든 이 서버를
연결하면, 도구 설명을 읽고 **"마인크래프트 실행해줘"** 같은 프롬프트를 데몬의
통합 명령 엔드포인트(`POST /api/agent/command`)로 전달한다.

## 구조

```
saba-chan-mcp/
  src/
    index.ts         # stdio MCP 서버 부팅
    config.ts        # env/토큰 해석 (DAEMON_API_URL, SABA_IPC_PORT, SABA_TOKEN_PATH, APPDATA)
    daemonClient.ts  # X-Saba-Token REST 호출 (얇은 래퍼, 새 로직 없음)
    tools.ts         # MCP 도구 3종 등록
  test/              # vitest 단위 테스트
```

이 래퍼는 **새 비즈니스 로직을 만들지 않는다.** 인스턴스 이름 해석과 동작
디스패치는 데몬의 `/api/agent/command` 핸들러가 담당하고, 래퍼는 MCP 요청을
그 엔드포인트로 옮기는 일만 한다.

## 도구

| 도구 | 용도 | 데몬 경로 |
|---|---|---|
| `list_servers` | 등록된 서버 목록/상태 조회 | `GET /api/servers` |
| `control_server` | 시작/중지/재시작/상태/콘솔/명령입력/RCON | `POST /api/agent/command` |
| `daemon_api_call` | 나머지 API 100% 커버용 제너릭 호출 | 임의 경로 |

### 예시 흐름 — "마인크래프트 실행해줘"

1. 에이전트가 `list_servers` 호출 → 'minecraft' 인스턴스 확인
2. `control_server(target="minecraft", action="start")` 호출
3. 데몬이 이름을 해석해 서버 시작

## 설정 (env)

| 변수 | 기본값 | 설명 |
|---|---|---|
| `DAEMON_API_URL` / `SABA_DAEMON_URL` | `http://127.0.0.1:57474` | 데몬 베이스 URL |
| `SABA_IPC_PORT` | `57474` | URL 미지정 시 포트 |
| `SABA_TOKEN_PATH` | 앱 데이터 경로의 `.ipc_token` | IPC 토큰 파일 |
| `SABA_DATA_DIR` | Windows: `%APPDATA%/saba-chan`, Unix: `$HOME/.config/saba-chan` | 데몬과 공유하는 데이터 경로 |
| `SABA_AUTH_DISABLED` | — | `1`이면 토큰 없이 동작(개발용) |

토큰은 이 프로세스만 보유하며 외부 에이전트에는 노출하지 않는다.
데몬 URL은 인증 정보와 쿼리가 없는 HTTP(S) 주소여야 한다.
직접 호출 경로는 `/api/` 아래로 제한하며 HTTP 리다이렉트는 거부한다.

## 빌드 / 실행 / 테스트

Node.js 22.12 이상을 사용한다. CI는 Node.js 24를 사용한다.

```bash
npm install
npm run build
npm test
node dist/index.js   # stdio MCP 서버
```

## MCP 클라이언트 등록 예시 (Claude Desktop)

```json
{
  "mcpServers": {
    "saba-chan": {
      "command": "node",
      "args": ["C:/Git/saba-chan/saba-chan-mcp/dist/index.js"]
    }
  }
}
```
