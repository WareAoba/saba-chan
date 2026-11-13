import type { SabaApi } from '@gui/api/createApi';
import type { ApiBody, Json, JsonObject, ModuleInfo, ServerInfo } from '@gui/types/daemon';
import type { TFunction } from 'i18next';

/** 웹이 쓰는 GUI 브리지 (window.api 와 같은 모양, 요청만 릴레이 세션으로 간다) */
export type WebApi = SabaApi;

/** 서버 목록 한 칸 (GET /api/servers) */
export type Server = ServerInfo;

/** 모듈 목록 한 칸 (GET /api/modules) */
export type Module = ModuleInfo;

/** 모듈 설정 값 맵 (module_settings 와 같다) */
export type SettingValues = Record<string, Json>;

/** 번역 함수 (react-i18next 의 t) */
export type TFn = TFunction;

export type { ApiBody, Json, JsonObject };

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
    data?: unknown;
    code?: string;
    error?: string;
}

/** 릴레이 세션이 밖에 내미는 것 (createSession 의 반환) */
export interface WebSession {
    request(method: string, path: string, body?: unknown, timeoutMs?: number): Promise<SessionReply>;
    stream(
        method: string,
        path: string,
        onChunk: (body: unknown) => void,
        onEnd: (reply: SessionReply) => void,
    ): (() => void) | null;
    extend(): void;
    leave(): void;
}

/** PC 에서 온 이벤트 */
export interface PcEvent {
    type: string;
    data: JsonObject;
}

// ── 릴레이 /relay/* 응답 (server-chan relay-server/src/services/relayApi.ts) ──

/** /relay/guilds/:g/... 어느 경로든 실패하면 오는 본문 */
export interface RelayErrorBody {
    error?: string;
    [key: string]: Json | undefined;
}

/** {ok, data} 또는 {ok:false, status, error, data} */
export type RelayResult<T> =
    | { ok: true; data: T }
    | { ok: false; status: number; error: string; data: RelayErrorBody | null };

/** GET /relay/me */
export interface RelayMe {
    userId: string;
    username: string | null;
    hasPc: boolean;
    pcs: { id: string; name: string; online: boolean }[];
}

/** GET /relay/guilds 항목의 can (accessOf) */
export interface RelayGuildCan {
    manage: boolean;
    channels: boolean;
    messages: boolean;
    members: boolean;
}

/** GET /relay/guilds 항목 */
export interface RelayGuild {
    id: string;
    name: string;
    icon: string | null;
    hasPc: boolean;
    can: RelayGuildCan;
}

/** GET·PATCH /relay/guilds/:g/settings 의 music (settingsView) */
export interface RelayMusicSettings {
    enabled: boolean;
    ui: boolean;
    queueLines: number;
    refreshInterval: number;
    normalize: boolean;
}

/** GET·PATCH /relay/guilds/:g/settings 의 tts (voiceOf) */
export interface RelayTtsSettings {
    volume: number;
    speed: number;
    pitch: number;
    names: 'always' | 'multi' | 'never';
    voiceChatOnly: boolean;
    keepRead: boolean;
    autoChannels: string[];
}

/** GET·PATCH /relay/guilds/:g/settings (settingsView) */
export interface RelayGuildSettings {
    locale: string;
    sleeping: boolean;
    sleepBlocks: string[] | null;
    bedtime: { start: string; end: string } | null;
    prefixes: string[] | null;
    translateCommands: boolean;
    slashPrefix: boolean;
    slashRootName: 'name' | 'word';
    privateQueries: boolean;
    bigEmoji: boolean;
    gameChatGuard: boolean;
    gameChatClearOnStop: boolean;
    memberNotice: boolean;
    busyNotice: boolean;
    music: RelayMusicSettings;
    tts: RelayTtsSettings;
}

/** PATCH /relay/guilds/:g/settings 본문 (settingsPatchSchema): 바꿀 키만 */
export type RelaySettingsPatch = Partial<Omit<RelayGuildSettings, 'music' | 'tts'>> & {
    music?: Partial<RelayMusicSettings>;
    tts?: Partial<RelayTtsSettings>;
};

/** GET·PUT /relay/guilds/:g/channels 의 고를 수 있는 채널 (AssignableChannel) */
export interface RelayAssignableChannel {
    id: string;
    name: string;
    category: string | null;
}

/** GET·PUT /relay/guilds/:g/channels (channelsView) */
export interface RelayChannels {
    guildId: string;
    chatbot: string[];
    music: string | null;
    notify: string | null;
    gameChat: string[];
    board: string[];
    botAvailable: boolean;
    channels: RelayAssignableChannel[];
    voiceChannels: RelayAssignableChannel[];
}

/** PUT /relay/guilds/:g/channels 본문 (nodeChannelsSchema): 준 키만 바꾼다 */
export interface RelayChannelsChange {
    chatbot?: string[];
    music?: string | null;
    notify?: string | null;
    gameChat?: string[];
    board?: string[];
}

/** GET /relay/guilds/:g/chatbot 의 settings (publicSettings 에서 channelIds·hasApiKey 를 뺀 것) */
export interface RelayChatbotSettings {
    enabled: boolean;
    provider: string;
    model: string;
    memo: string;
    effort: 'low' | 'medium' | 'high';
    ambient: boolean;
    ambientMinutes: number;
    ambientEffort: 'low' | 'medium' | 'high';
    confirmLevel: 'all' | 'caution' | 'danger' | 'none';
    memory: boolean;
    moderation: boolean;
    webSearch: boolean;
    tools?: string[];
    disabledTools: string[];
    tokenLimit: number | null;
    balanceAlert: number | null;
    effectiveModel: string;
}

/** GET·PATCH /relay/guilds/:g/chatbot */
export interface RelayChatbot {
    settings: Partial<RelayChatbotSettings>;
    hasApiKey: boolean;
}

/** PATCH /relay/guilds/:g/chatbot 본문 (chatbotPatchSchema) */
export interface RelayChatbotPatch {
    settings?: Partial<RelayChatbotSettings>;
    apiKey?: string | null;
}

/** DELETE /relay/guilds/:g/chatbot/memory */
export interface RelaySuccess {
    success: boolean;
}

/** 지우기·자동 청소 대상 (CLEAR_TARGETS) */
export type RelayClearTarget = 'all' | 'humans' | 'self' | 'other_bots' | 'bots';

/** 제재 종류 (SANCTION_KINDS) */
export type RelaySanctionKind = 'chat' | 'voice' | 'ban' | 'all';

/** GET /relay/guilds/:g/moderation 의 채널, PATCH .../moderation/channels/:c 의 답 (channelView) */
export interface RelayModChannel {
    id: string;
    name: string;
    category: string | null;
    topic: string | null;
    slowmode: number;
    locked: { until: string | null } | null;
    slowUntil: string | null;
    autoClear: number | null;
    autoClearDaily: string | null;
    autoClearTarget: RelayClearTarget;
    clearing: { deleted: number; oldestAt: string | null } | null;
}

/** GET /relay/guilds/:g/moderation 의 can: 봇이 가진 권한 (moderationView) */
export interface RelayModCan {
    manageMessages: boolean;
    manageChannels: boolean;
    manageRoles: boolean;
    moderateMembers: boolean;
    muteMembers: boolean;
    banMembers: boolean;
}

/** GET /relay/guilds/:g/moderation 의 mutes 항목 */
export interface RelayModMute {
    userId: string;
    name: string;
    avatarUrl: string;
    forever: boolean;
    until: string | null;
}

/** GET /relay/guilds/:g/moderation 의 sanctions 항목 */
export interface RelayModSanction {
    userId: string;
    name: string;
    kind: RelaySanctionKind;
    until: string | null;
    inGuild: boolean;
}

/** GET /relay/guilds/:g/moderation (moderationView) */
export interface RelayModeration {
    guildId: string;
    botAvailable: boolean;
    can: RelayModCan | null;
    channels: RelayModChannel[];
    mutes: RelayModMute[];
    sanctions: RelayModSanction[];
}

/** GET /relay/guilds/:g/moderation/members 항목 */
export interface RelayModMember {
    id: string;
    name: string;
    username: string;
    avatarUrl: string;
}

/** PATCH /relay/guilds/:g/moderation/channels/:c 본문 (modChannelSchema) */
export interface RelayModChannelEdit {
    name?: string;
    topic?: string | null;
    slowmode?: number;
    lock?: 'off' | 'forever' | number;
    autoClear?: number | null;
    slowFor?: number;
    autoClearDaily?: string;
    autoClearTarget?: RelayClearTarget;
}

/** POST /relay/guilds/:g/moderation/channels/:c/clear 본문 (modClearSchema) */
export interface RelayModClear {
    count?: number;
    authorId?: string;
    sinceMinutes?: number;
    target?: RelayClearTarget;
    from?: number;
    to?: number;
}

/** POST /relay/guilds/:g/moderation/channels/:c/clear 의 답 (clearModChannel) */
export interface RelayModClearResult {
    deleted: number;
    kept: number;
    olderLeft: boolean;
    job: boolean;
}

/** DELETE /relay/guilds/:g/moderation/channels/:c/clear 의 답 (stopModClear) */
export interface RelayModStopResult {
    stopping: boolean;
}

/** PUT /relay/guilds/:g/moderation/mutes/:u, .../sanctions/:u 의 답 (muteModMember·sanctionModMember) */
export interface RelayModActionResult {
    result: string;
    until?: string | null;
}

/** PUT /relay/guilds/:g/moderation/mutes/:u 본문 (modMuteSchema) */
export interface RelayModMuteBody {
    duration: 'off' | 'forever' | number;
    reason?: string;
}

/** PUT /relay/guilds/:g/moderation/sanctions/:u 본문 (modSanctionSchema) */
export interface RelayModSanctionBody {
    kind: RelaySanctionKind;
    duration: 'off' | 'forever' | number;
    reason?: string;
    purgeSeconds?: 0 | 3600 | 86400 | 604800;
}

/** 서버 상세의 탭(패널)이 받는 것 */
export interface ServerPanelProps {
    api: WebApi;
    server: Server;
    moduleData?: Partial<Module> | null;
    running: boolean;
}
