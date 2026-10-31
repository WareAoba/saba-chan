/**
 * 릴레이 API — 웹 세션에서 /relay/ 로 시작하는 요청은 PC 가 아니라 릴레이가 답한다
 * (server-chan relay-server/src/services/relayApi.ts, 계약은 docs/architecture/05 §4.4(f)).
 * PC 없는 사람도 쓰는 디스코드 설정·채널 배정·말하는 사바쨩·서버 관리. 답은 {ok, data} 또는 {ok:false, status, error, data}.
 */

const guild = (g, rest = '') => `/relay/guilds/${encodeURIComponent(g)}${rest}`;
const id = encodeURIComponent;

export function createRelay(session) {
    const call = async (method, path, body) => {
        const res = await session.request(method, path, body);
        if (res?.status >= 200 && res.status < 300) return { ok: true, data: res.data };
        const data = res?.data ?? null;
        return { ok: false, status: res?.status ?? 0, error: data?.error || res?.code || 'error', data };
    };
    return {
        me: () => call('GET', '/relay/me'),
        guilds: () => call('GET', '/relay/guilds'),
        settings: (g) => call('GET', guild(g, '/settings')),
        saveSettings: (g, patch) => call('PATCH', guild(g, '/settings'), patch),
        channels: (g) => call('GET', guild(g, '/channels')),
        saveChannels: (g, change) => call('PUT', guild(g, '/channels'), change),
        chatbot: (g) => call('GET', guild(g, '/chatbot')),
        saveChatbot: (g, body) => call('PATCH', guild(g, '/chatbot'), body),
        clearMemory: (g) => call('DELETE', guild(g, '/chatbot/memory')),
        moderation: (g) => call('GET', guild(g, '/moderation')),
        modChannel: (g, c, edit) => call('PATCH', guild(g, `/moderation/channels/${id(c)}`), edit),
        modClear: (g, c, body) => call('POST', guild(g, `/moderation/channels/${id(c)}/clear`), body),
        modMute: (g, u, body) => call('PUT', guild(g, `/moderation/mutes/${id(u)}`), body),
    };
}
