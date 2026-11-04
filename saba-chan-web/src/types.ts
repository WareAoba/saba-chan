/**
 * 릴레이·데몬이 주는 JSON 은 모양이 모듈·버전마다 달라 화면 코드가 느슨하게 읽는다.
 * 그런 값에만 이 별칭을 쓴다 (모양이 정해진 값은 따로 타입을 둔다).
 */
// biome-ignore lint/suspicious/noExplicitAny: 릴레이·데몬 JSON 은 모듈마다 모양이 달라 정해 둘 수 없다
export type Loose = any;

/** 릴레이 웹 세션 상태 (session.ts) */
export interface SessionState {
    phase: 'connecting' | 'ready' | 'reconnecting' | 'closed';
    reason: string | null;
    expiresAt: number | null;
    hostId: string | null;
    warning: boolean;
}

/** daemon_request 와 같은 모양의 답: {status, data} 또는 연결 실패 {status:0, code} */
export interface SessionReply {
    status: number;
    data?: Loose;
    code?: string;
    error?: string;
}

/** 릴레이 세션이 밖에 내미는 것 (createSession 의 반환) */
export interface WebSession {
    request(method: string, path: string, body?: unknown, timeoutMs?: number): Promise<SessionReply>;
    extend(): void;
    leave(): void;
}

/** PC 에서 온 이벤트 */
export interface PcEvent {
    type: string;
    data: Loose;
}
