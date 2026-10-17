import { createServer, type RequestListener, type Server } from "node:http";
import { afterEach, describe, expect, it } from "vitest";
import { DaemonClient } from "../src/daemonClient.js";

const servers: Server[] = [];

async function serve(handler: RequestListener): Promise<string> {
  const server = createServer(handler);
  servers.push(server);
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("No test port");
  return `http://127.0.0.1:${address.port}`;
}

describe("DaemonClient request boundaries", () => {
  afterEach(async () => {
    await Promise.all(servers.splice(0).map((server) => new Promise<void>((resolve, reject) => {
      server.close((error) => error ? reject(error) : resolve());
      server.closeAllConnections();
    })));
  });

  it("실제 데몬 HTTP 응답을 읽고 토큰을 전달한다", async () => {
    const baseUrl = await serve((request, response) => {
      response.end(JSON.stringify({ token: request.headers["x-saba-token"] }));
    });
    const client = new DaemonClient({ baseUrl, token: "test-token" });
    expect((await client.listServers()).body).toEqual({ token: "test-token" });
  });

  it("잘못된 인증 헤더의 토큰을 오류 메시지에 노출하지 않는다", async () => {
    const baseUrl = await serve((_request, response) => response.end("{}"));
    const client = new DaemonClient({ baseUrl, token: "fake-secret-token\ninjected" });
    await expect(client.listServers()).rejects.not.toThrow("fake-secret-token");
  });

  it("리다이렉트 대상에 IPC 토큰을 전달하지 않는다", async () => {
    let receivedToken: string | undefined;
    const destination = await serve((request, response) => {
      receivedToken = request.headers["x-saba-token"] as string | undefined;
      response.end("{}");
    });
    const baseUrl = await serve((_request, response) => {
      response.writeHead(302, { Location: `${destination}/api/servers` });
      response.end();
    });
    const client = new DaemonClient({ baseUrl, token: "test-token" });
    await expect(client.listServers()).rejects.toThrow();
    expect(receivedToken).toBeUndefined();
  });

  it.each(["/api/../private", "//example.invalid/api/servers", "/api/..\\private"])(
    "API 외부 경로를 전송 전에 거부한다: %s",
    async (apiPath) => {
      let requests = 0;
      const baseUrl = await serve((_request, response) => {
        requests += 1;
        response.end("{}");
      });
      const client = new DaemonClient({ baseUrl, token: "test-token" });
      await expect(client.request("GET", apiPath)).rejects.toThrow(/API/);
      expect(requests).toBe(0);
    },
  );
});
