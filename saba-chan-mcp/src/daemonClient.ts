/**
 * 데몬 REST API 클라이언트.
 *
 * 새 비즈니스 로직을 만들지 않는다. 데몬 HTTP API를 호출하고 응답을
 * 그대로(또는 최소 정규화하여) 반환하는 얇은 래퍼다. 인증 토큰은 이
 * 프로세스만 보유하며, 외부 에이전트에는 노출하지 않는다.
 */
import type { DaemonConfig } from "./config.js";

export interface DaemonResponse {
  ok: boolean;
  status: number;
  body: unknown;
}

export class DaemonClient {
  constructor(private readonly config: DaemonConfig) {}

  private headers(): Record<string, string> {
    const h: Record<string, string> = { "Content-Type": "application/json" };
    if (this.config.token) {
      h["X-Saba-Token"] = this.config.token;
    }
    return h;
  }

  /**
   * 임의 HTTP 메서드/경로 호출. 데몬 API 100% 커버용 이스케이프 해치.
   * @param method GET/POST/PUT/PATCH/DELETE
   * @param apiPath `/api/...` 로 시작하는 경로
   * @param body JSON 직렬화할 본문 (선택)
   */
  async request(
    method: string,
    apiPath: string,
    body?: unknown,
  ): Promise<DaemonResponse> {
    const normalizedPath = apiPath.startsWith("/") ? apiPath : `/${apiPath}`;
    const base = new URL(this.config.baseUrl);
    const url = new URL(`${this.config.baseUrl}${normalizedPath}`);
    if (url.origin !== base.origin || !url.pathname.startsWith("/api/") || url.hash || apiPath.includes("\\")) {
      throw new Error("데몬 API 경로는 /api/로 시작해야 합니다.");
    }
    const requestMethod = method.toUpperCase();
    const init: RequestInit = {
      method: requestMethod,
      headers: this.headers(),
      redirect: "error",
    };
    if (body !== undefined && requestMethod !== "GET" && requestMethod !== "HEAD") {
      init.body = JSON.stringify(body);
    }

    let res: Response;
    try {
      res = await fetch(url.href, init);
    } catch {
      throw new Error("데몬에 연결할 수 없습니다. 데몬이 실행 중인지 확인하세요.");
    }

    const text = await res.text();
    let parsed: unknown = text;
    if (text.length > 0) {
      try {
        parsed = JSON.parse(text);
      } catch {
        // 비-JSON 응답은 원문 문자열 그대로 반환
        parsed = text;
      }
    }

    return { ok: res.ok, status: res.status, body: parsed };
  }

  /** GET /api/servers — 등록된 서버 인스턴스와 런타임 상태. */
  listServers(): Promise<DaemonResponse> {
    return this.request("GET", "/api/servers");
  }

  /** POST /api/agent/command — 통합 명령 진입점. */
  agentCommand(payload: {
    target: string;
    action: string;
    command?: string;
    force?: boolean;
  }): Promise<DaemonResponse> {
    return this.request("POST", "/api/agent/command", payload);
  }
}
