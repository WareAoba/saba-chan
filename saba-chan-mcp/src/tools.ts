/**
 * MCP 도구 등록.
 *
 * 데몬 API 93개를 그대로 노출하지 않는다. 일상 시나리오("마인크래프트
 * 실행해줘")를 위한 큐레이션된 핵심 도구 + 나머지 100% 커버용 제너릭
 * 이스케이프 해치로 구성한다.
 */
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import type { DaemonClient, DaemonResponse } from "./daemonClient.js";

/** 데몬 응답을 MCP 텍스트 콘텐츠로 변환. */
function toToolResult(res: DaemonResponse) {
  const text =
    typeof res.body === "string"
      ? res.body
      : JSON.stringify(res.body, null, 2);
  return {
    content: [{ type: "text" as const, text }],
    isError: !res.ok,
  };
}

function errorResult(message: string) {
  return {
    content: [{ type: "text" as const, text: message }],
    isError: true,
  };
}

export function registerTools(server: McpServer, client: DaemonClient): void {
  // ── 1. 서버 목록 조회 ──────────────────────────────────
  server.registerTool(
    "list_servers",
    {
      title: "게임 서버 목록 조회",
      description:
        "saba-chan에 등록된 모든 게임 서버 인스턴스와 현재 실행 상태(running/stopped 등)를 반환한다. " +
        "사용자가 특정 서버(예: '마인크래프트')를 언급하면, 먼저 이 도구를 호출해 정확한 서버 이름을 확인한 뒤 control_server를 사용하라.",
      inputSchema: {},
    },
    async () => {
      try {
        return toToolResult(await client.listServers());
      } catch (err) {
        return errorResult((err as Error).message);
      }
    },
  );

  // ── 2. 서버 제어 (핵심 래퍼) ───────────────────────────
  server.registerTool(
    "control_server",
    {
      title: "게임 서버 제어 / 콘솔 명령 입력",
      description:
        "게임 서버를 시작/중지/재시작하거나, 실행 중인 서버 콘솔에 텍스트 명령을 직접 입력한다. " +
        "예: 사용자가 '마인크래프트 실행해줘'라고 하면 target='minecraft', action='start' 로 호출하라. " +
        "콘솔에 명령을 보내려면 action='command' 와 함께 command 필드에 보낼 텍스트 원문을 그대로 넣어라(예: command='say hello'). " +
        "target은 서버 이름(대소문자 무시) 또는 인스턴스 id 둘 다 허용된다. " +
        "stop/restart 등 파괴적/중단 동작은 사용자의 의도가 명확할 때만 실행하라.",
      inputSchema: {
        target: z
          .string()
          .describe("대상 서버 이름(대소문자 무시) 또는 인스턴스 UUID"),
        action: z
          .enum([
            "start",
            "stop",
            "restart",
            "status",
            "console",
            "command",
            "rcon",
          ])
          .describe(
            "수행할 동작. start=시작, stop=중지, restart=재시작, status=상태조회, " +
              "console=콘솔 출력 읽기, command=콘솔(stdin)에 텍스트 입력, rcon=RCON 명령 실행",
          ),
        command: z
          .string()
          .optional()
          .describe("action이 command 또는 rcon일 때 서버에 보낼 텍스트 명령 원문"),
        force: z
          .boolean()
          .optional()
          .describe("stop/restart 시 강제 종료 여부 (기본 false)"),
      },
    },
    async (args) => {
      try {
        return toToolResult(
          await client.agentCommand({
            target: args.target,
            action: args.action,
            command: args.command,
            force: args.force,
          }),
        );
      } catch (err) {
        return errorResult((err as Error).message);
      }
    },
  );

  // ── 3. 제너릭 데몬 API 호출 (이스케이프 해치) ──────────
  server.registerTool(
    "daemon_api_call",
    {
      title: "데몬 API 직접 호출 (고급)",
      description:
        "큐레이션된 도구로 처리할 수 없는 saba-chan 데몬 REST API를 직접 호출한다. " +
        "일반적인 서버 제어에는 control_server를 우선 사용하고, 인스턴스 생성/모듈 설치/설정 변경 등 " +
        "고급 작업이 필요할 때만 이 도구를 사용하라. path는 '/api/...' 형식이어야 한다.",
      inputSchema: {
        method: z
          .enum(["GET", "POST", "PUT", "PATCH", "DELETE"])
          .describe("HTTP 메서드"),
        path: z
          .string()
          .describe("API 경로. 예: /api/instances, /api/instance/<id>/console"),
        body: z
          .record(z.unknown())
          .optional()
          .describe("요청 본문 JSON 객체 (GET에는 무시됨)"),
      },
    },
    async (args) => {
      try {
        return toToolResult(
          await client.request(args.method, args.path, args.body),
        );
      } catch (err) {
        return errorResult((err as Error).message);
      }
    },
  );
}
