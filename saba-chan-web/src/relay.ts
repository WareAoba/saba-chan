import type { Loose, WebSession } from './types';
/**
 * 릴레이 API — 웹 세션에서 /relay/ 로 시작하는 요청은 PC 가 아니라 릴레이가 답한다
 * (server-chan relay-server/src/services/relayApi.ts, 계약은 docs/architecture/05 §4.4(f)).
 * PC 없는 사람도 쓰는 디스코드 설정·채널 배정·말하는 사바쨩·서버 관리. 답은 {ok, data} 또는 {ok:false, status, error, data}.
 */

const guild = (g: string, rest = '') => `/relay/guilds/${encodeURIComponent(g)}${rest}`;
const id = encodeURIComponent;

/** {ok, data} 또는 {ok:false, status, error, data} */
export interface RelayResult {
    ok: boolean;
    status?: number;
    error?: string;
    data: Loose;
}

export function createRelay(session: Pick<WebSession, 'request'>) {
    const call = async (method: string, path: string, body?: unknown): Promise<RelayResult> => {
        const res = await session.request(method, path, body);
        if (res?.status >= 200 && res.status < 300) return { ok: true, data: res.data };
        const data = res?.data ?? null;
        return { ok: false, status: res?.status ?? 0, error: data?.error || res?.code || 'error', data };
    };
    return {
        me: () => call('GET', '/relay/me'),
        guilds: () => call('GET', '/relay/guilds'),
        settings: (g: string) => call('GET', guild(g, '/settings')),
        saveSettings: (g: string, patch: Loose) => call('PATCH', guild(g, '/settings'), patch),
        channels: (g: string) => call('GET', guild(g, '/channels')),
        saveChannels: (g: string, change: Loose) => call('PUT', guild(g, '/channels'), change),
        chatbot: (g: string) => call('GET', guild(g, '/chatbot')),
        saveChatbot: (g: string, body: Loose) => call('PATCH', guild(g, '/chatbot'), body),
        clearMemory: (g: string) => call('DELETE', guild(g, '/chatbot/memory')),
        moderation: (g: string) => call('GET', guild(g, '/moderation')),
        modChannel: (g: string, c: string, edit: Loose) => call('PATCH', guild(g, `/moderation/channels/${id(c)}`), edit),
        modClear: (g: string, c: string, body: Loose) => call('POST', guild(g, `/moderation/channels/${id(c)}/clear`), body),
        modClearStop: (g: string, c: string) => call('DELETE', guild(g, `/moderation/channels/${id(c)}/clear`)),
        modMute: (g: string, u: string, body: Loose) => call('PUT', guild(g, `/moderation/mutes/${id(u)}`), body),
        modSanction: (g: string, u: string, body: Loose) => call('PUT', guild(g, `/moderation/sanctions/${id(u)}`), body),
        modMembers: (g: string) => call('GET', guild(g, '/moderation/members')),
    };
}
