import type {
    RelayChannels,
    RelayChannelsChange,
    RelayChatbot,
    RelayChatbotPatch,
    RelayErrorBody,
    RelayGuild,
    RelayGuildSettings,
    RelayMe,
    RelayModActionResult,
    RelayModChannel,
    RelayModChannelEdit,
    RelayModClear,
    RelayModClearResult,
    RelayModeration,
    RelayModMember,
    RelayModMuteBody,
    RelayModSanctionBody,
    RelayModStopResult,
    RelayResult,
    RelaySettingsPatch,
    RelaySuccess,
    WebSession,
} from './types';
/**
 * 릴레이 API — 웹 세션에서 /relay/ 로 시작하는 요청은 PC 가 아니라 릴레이가 답한다
 * (server-chan relay-server/src/services/relayApi.ts, 계약은 docs/architecture/05 §4.4(f)).
 * PC 없는 사람도 쓰는 디스코드 설정·채널 배정·말하는 사바쨩·서버 관리. 답은 {ok, data} 또는 {ok:false, status, error, data}.
 */

const guild = (g: string, rest = '') => `/relay/guilds/${encodeURIComponent(g)}${rest}`;
const id = encodeURIComponent;

export function createRelay(session: Pick<WebSession, 'request'>) {
    const call = async <T>(method: string, path: string, body?: unknown): Promise<RelayResult<T>> => {
        const res = await session.request(method, path, body);
        if (res?.status >= 200 && res.status < 300) return { ok: true, data: res.data as T };
        const data = (res?.data ?? null) as RelayErrorBody | null;
        return { ok: false, status: res?.status ?? 0, error: data?.error || res?.code || 'error', data };
    };
    return {
        me: () => call<RelayMe>('GET', '/relay/me'),
        guilds: () => call<RelayGuild[]>('GET', '/relay/guilds'),
        settings: (g: string) => call<RelayGuildSettings>('GET', guild(g, '/settings')),
        saveSettings: (g: string, patch: RelaySettingsPatch) => call<RelayGuildSettings>('PATCH', guild(g, '/settings'), patch),
        channels: (g: string) => call<RelayChannels>('GET', guild(g, '/channels')),
        saveChannels: (g: string, change: RelayChannelsChange) => call<RelayChannels>('PUT', guild(g, '/channels'), change),
        chatbot: (g: string) => call<RelayChatbot>('GET', guild(g, '/chatbot')),
        saveChatbot: (g: string, body: RelayChatbotPatch) => call<RelayChatbot>('PATCH', guild(g, '/chatbot'), body),
        clearMemory: (g: string) => call<RelaySuccess>('DELETE', guild(g, '/chatbot/memory')),
        moderation: (g: string) => call<RelayModeration>('GET', guild(g, '/moderation')),
        modChannel: (g: string, c: string, edit: RelayModChannelEdit) => call<RelayModChannel>('PATCH', guild(g, `/moderation/channels/${id(c)}`), edit),
        modClear: (g: string, c: string, body: RelayModClear) => call<RelayModClearResult>('POST', guild(g, `/moderation/channels/${id(c)}/clear`), body),
        modClearStop: (g: string, c: string) => call<RelayModStopResult>('DELETE', guild(g, `/moderation/channels/${id(c)}/clear`)),
        modMute: (g: string, u: string, body: RelayModMuteBody) => call<RelayModActionResult>('PUT', guild(g, `/moderation/mutes/${id(u)}`), body),
        modSanction: (g: string, u: string, body: RelayModSanctionBody) => call<RelayModActionResult>('PUT', guild(g, `/moderation/sanctions/${id(u)}`), body),
        modMembers: (g: string) => call<RelayModMember[]>('GET', guild(g, '/moderation/members')),
    };
}
