import { afterEach, describe, expect, it, vi } from "vitest";
import { DaemonClient } from "../src/daemonClient.js";

const config = { baseUrl: "http://127.0.0.1:57474", token: "test-token" };

function mockFetch(status: number, body: unknown) {
  return vi.fn(async () =>
    new Response(JSON.stringify(body), {
      status,
      headers: { "Content-Type": "application/json" },
    }),
  );
}

describe("DaemonClient", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("control_server는 /api/agent/command 로 X-Saba-Token 헤더와 함께 POST한다", async () => {
    const fetchMock = mockFetch(200, { success: true });
    vi.stubGlobal("fetch", fetchMock);

    const client = new DaemonClient(config);
    const res = await client.agentCommand({ target: "minecraft", action: "start" });

    expect(res.ok).toBe(true);
    expect(fetchMock).toHaveBeenCalledOnce();
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("http://127.0.0.1:57474/api/agent/command");
    expect(init.method).toBe("POST");
    expect((init.headers as Record<string, string>)["X-Saba-Token"]).toBe("test-token");
    expect(JSON.parse(init.body as string)).toEqual({
      target: "minecraft",
      action: "start",
    });
  });

  it("토큰이 null이면 X-Saba-Token 헤더를 보내지 않는다", async () => {
    const fetchMock = mockFetch(200, {});
    vi.stubGlobal("fetch", fetchMock);

    const client = new DaemonClient({ ...config, token: null });
    await client.listServers();

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect((init.headers as Record<string, string>)["X-Saba-Token"]).toBeUndefined();
  });

  it("request는 비-2xx 응답에 ok=false 와 파싱된 본문을 반환한다", async () => {
    const fetchMock = mockFetch(404, { error_code: "instance_not_found" });
    vi.stubGlobal("fetch", fetchMock);

    const client = new DaemonClient(config);
    const res = await client.request("POST", "/api/agent/command", { target: "x", action: "start" });

    expect(res.ok).toBe(false);
    expect(res.status).toBe(404);
    expect(res.body).toEqual({ error_code: "instance_not_found" });
  });

  it("GET 요청은 본문을 직렬화하지 않는다", async () => {
    const fetchMock = mockFetch(200, { servers: [] });
    vi.stubGlobal("fetch", fetchMock);

    const client = new DaemonClient(config);
    await client.request("GET", "/api/servers", { ignored: true });

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(init.body).toBeUndefined();
  });

  it("연결 실패 시 안내 메시지를 담아 throw 한다", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new Error("ECONNREFUSED");
      }),
    );

    const client = new DaemonClient(config);
    await expect(client.listServers()).rejects.toThrow(/데몬에 연결할 수 없습니다/);
  });
});
