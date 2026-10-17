#!/usr/bin/env node
/**
 * saba-chan-mcp — saba-chan 데몬용 MCP(Model Context Protocol) 서버.
 *
 * 어떤 MCP 호환 AI 에이전트든 이 프로세스를 stdio로 띄우면, 도구 설명을
 * 읽고 "마인크래프트 실행해줘" 같은 자연어 의도를 데몬의 통합 명령
 * 엔드포인트(/api/agent/command)로 전달할 수 있다.
 *
 * 에러는 stderr로 출력하여 상위 프로세스가 수집할 수 있게 한다(삼키지 않음).
 */
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { loadDaemonConfig } from "./config.js";
import { DaemonClient } from "./daemonClient.js";
import { registerTools } from "./tools.js";

async function main(): Promise<void> {
  const config = loadDaemonConfig();
  const client = new DaemonClient(config);

  const server = new McpServer({
    name: "saba-chan-mcp",
    version: "0.1.0",
  });

  registerTools(server, client);

  const transport = new StdioServerTransport();
  await server.connect(transport);

  // stdio MCP 서버는 연결 종료 시까지 살아 있다.
  process.stderr.write(
    `[saba-chan-mcp] 연결됨. 데몬: ${config.baseUrl} (인증: ${
      config.token ? "활성" : "비활성"
    })\n`,
  );
}

main().catch((err) => {
  process.stderr.write(`[saba-chan-mcp] 치명적 오류: ${err.message}\n`);
  process.exit(1);
});
