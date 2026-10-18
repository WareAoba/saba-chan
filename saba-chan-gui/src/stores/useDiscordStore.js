import { create } from 'zustand';
import { readPrefixes } from '../components/discord/prefixes';
import { createTranslateError, safeShowToast } from '../utils/helpers';

// i18n translate function — set after store creation
let _t = (key, fallback) => fallback || key;
let _translateError = (msg) => msg;

export const setDiscordI18n = (t) => {
    _t = t;
    _translateError = createTranslateError(t);
};

const RELAY_URL_FALLBACK = 'https://saba-chan.online';

// bot-config 최상위 키 ↔ 스토어 필드. 데몬은 최상위 키 단위로 통째로 바꾸므로
// 바뀐 키만 보낸다: 다른 경로(CLI·원격)가 그사이 바꾼 값을 옛 스냅샷으로 되돌리지 않게
const BOT_KEYS = {
    prefixes: ['discordPrefixes'],
    cloud: ['discordCloudRelayUrl', 'discordCloudHostId'],
    moduleAliases: ['discordModuleAliases'],
    commandAliases: ['discordCommandAliases'],
    nodeSettings: ['nodeSettings'],
    cloudNodes: ['cloudNodes'],
    cloudMembers: ['cloudMembers'],
};
// 마지막으로 데몬과 맞춘 값 (로드·저장 성공 때 갱신). 여기와 다른 필드만 바뀐 것으로 본다
let synced = {};
const sameValue = (a, b) => a === b || JSON.stringify(a) === JSON.stringify(b);
const markSynced = (state, fields) => {
    for (const f of fields) synced[f] = state[f];
};
const ALL_FIELDS = Object.values(BOT_KEYS).flat();
// 상태 확인 세대: 연결 대상이 바뀌거나 폴링이 멈추면 올려서, 늦게 온 옛 응답을 버린다
let statusGeneration = 0;

// 디스코드 봇 본체는 클라우드(릴레이 서버)에서 돌고, 로컬에서는 데몬 내장
// 릴레이 에이전트가 명령을 받아 처리한다. 이 스토어는 bot-config와
// 에이전트/릴레이 상태만 다룬다. 에이전트는 노드 토큰이 있으면 데몬이 자동 시작한다.
const initialState = () => ({
    // ── Bot config (bot-config.json, 데몬이 SSOT) ──
    // 비어 있으면 릴레이가 언어별 기본 접두사를 쓴다
    discordPrefixes: [],
    discordModuleAliases: {},
    discordCommandAliases: {},
    discordCloudRelayUrl: '',
    discordCloudHostId: '',
    nodeSettings: {},
    cloudNodes: [],
    cloudMembers: {},

    // ── Runtime status ──
    // stopped: 에이전트 정지 / connecting: 에이전트 실행 중, 릴레이 미확인 / running: 둘 다 OK
    discordBotStatus: 'stopped',
    // 다른 PC가 같은 계정으로 페어링해서 이 PC의 에이전트가 멈췄다
    discordAgentReplaced: false,
    botStatusReady: false,
    relayConnected: false,
    relayConnecting: false,

    // ── Internal ──
    _settingsReady: false,
    _botConfigLoaded: false,
    _statusInterval: null,
    _statusTimeout: null,
});

export const useDiscordStore = create((set, get) => ({
    ...initialState(),

    update: (partial) => {
        const state = get();
        const targetChanged = ['discordCloudHostId', 'discordCloudRelayUrl'].some(
            (k) => k in partial && partial[k] !== state[k],
        );
        if (!targetChanged) return set(partial);
        // 연결 대상이 바뀌면 옛 호스트의 확인 결과는 무효 (다음 폴링까지 잘못된 상태를 보이지 않게)
        statusGeneration += 1;
        set({ ...partial, relayConnected: false, relayConnecting: false });
        if (get()._statusInterval) get().checkStatus();
    },

    loadConfig: async () => {
        const isTest = process.env.NODE_ENV === 'test' || typeof jest !== 'undefined';
        const maxAttempts = isTest ? 1 : 5;
        const retryDelay = 800;

        for (let attempt = 1; attempt <= maxAttempts; attempt++) {
            try {
                const botCfg = await window.api.botConfigLoad();
                if (!botCfg) continue;

                const patch = {
                    discordPrefixes: readPrefixes(botCfg),
                    discordModuleAliases: botCfg.moduleAliases || {},
                    discordCommandAliases: botCfg.commandAliases || {},
                    discordCloudRelayUrl: botCfg.cloud?.relayUrl || '',
                    discordCloudHostId: botCfg.cloud?.hostId || '',
                    _botConfigLoaded: true,
                };
                if (botCfg.nodeSettings && typeof botCfg.nodeSettings === 'object') {
                    patch.nodeSettings = botCfg.nodeSettings;
                }
                if (Array.isArray(botCfg.cloudNodes)) patch.cloudNodes = botCfg.cloudNodes;
                if (botCfg.cloudMembers && typeof botCfg.cloudMembers === 'object')
                    patch.cloudMembers = botCfg.cloudMembers;

                set(patch);
                synced = {};
                markSynced(get(), ALL_FIELDS);
                return;
            } catch (err) {
                if (attempt === maxAttempts) {
                    console.error('Failed to load bot config after retries:', err);
                } else {
                    if (!isTest) console.warn(`[Settings] Bot config load attempt ${attempt} failed, retrying...`);
                    await new Promise((r) => setTimeout(r, retryDelay));
                }
            }
        }
    },

    saveConfig: async () => {
        const state = get();
        try {
            // 처음 로드에 실패했다면 스토어 값은 기본값이다. 데몬을 읽을 수 없으면 저장하지 않는다
            if (!state._botConfigLoaded && !(await window.api.botConfigLoad())) {
                throw new Error('bot config unavailable; refusing to overwrite it with defaults');
            }
            const initial = initialState();
            const base = (f) => (f in synced ? synced[f] : initial[f]);
            const changed = Object.keys(BOT_KEYS).filter((k) => BOT_KEYS[k].some((f) => !sameValue(state[f], base(f))));
            if (changed.length === 0) return true;

            const payload = {};
            for (const k of changed) {
                if (k === 'cloud') {
                    // relayUrl·hostId는 한 키로 간다. 바꾸지 않은 쪽은 데몬의 지금 값을 쓴다
                    const fresh = (await window.api.botConfigLoad().catch(() => null))?.cloud || {};
                    const pickCloud = (f, sub) =>
                        sameValue(state[f], base(f)) && typeof fresh[sub] === 'string' ? fresh[sub] : state[f];
                    payload.cloud = {
                        relayUrl: pickCloud('discordCloudRelayUrl', 'relayUrl'),
                        hostId: pickCloud('discordCloudHostId', 'hostId'),
                    };
                } else {
                    payload[k] = state[BOT_KEYS[k][0]];
                }
            }
            const res = await window.api.botConfigSave(payload);
            if (res?.error) {
                console.error('[Settings] Failed to save bot config:', res.error);
                safeShowToast(_t('settings.save_error'), 'error');
                return false;
            }
            markSynced(
                state,
                changed.flatMap((k) => BOT_KEYS[k]),
            );
            return true;
        } catch (error) {
            console.error('[Settings] Failed to save bot config:', error);
            safeShowToast(_t('settings.save_error'), 'error');
            return false;
        }
    },

    /**
     * 데몬 전용 API로 이미 저장한 길드 값을 스토어에 반영한다. 자동 저장이 다시 보내지 않게 동기화 기준도 같이 바꾼다
     */
    applySavedNodeValue: (guildId, key, value) => {
        const put = (all) => {
            const node = { ...(all?.[guildId] || {}) };
            if (value == null) delete node[key];
            else node[key] = value;
            return { ...(all || {}), [guildId]: node };
        };
        synced.nodeSettings = put(synced.nodeSettings ?? initialState().nodeSettings);
        set({ nodeSettings: put(get().nodeSettings) });
    },

    startBot: async () => {
        try {
            const result = await window.api.discordBotStart();
            if (result?.error) {
                safeShowToast(
                    _t('discord_bot.start_failed_toast', { error: _translateError(result.error) }),
                    'error',
                    4000,
                );
            } else {
                set({ discordBotStatus: 'connecting' });
                safeShowToast(_t('discord_bot.started_toast'), 'discord', 3000);
                get().checkStatus();
            }
        } catch (e) {
            safeShowToast(_t('discord_bot.start_error_toast', { error: _translateError(e.message) }), 'error', 4000);
        }
    },

    stopBot: async () => {
        try {
            const result = await window.api.discordBotStop();
            if (result?.error) {
                safeShowToast(
                    _t('discord_bot.stop_failed_toast', { error: _translateError(result.error) }),
                    'error',
                    4000,
                );
            } else {
                set({ discordBotStatus: 'stopped', relayConnected: false });
                safeShowToast(_t('discord_bot.stopped_toast'), 'discord', 3000);
            }
        } catch (e) {
            safeShowToast(_t('discord_bot.stop_error_toast', { error: _translateError(e.message) }), 'error', 4000);
        }
    },

    checkStatus: async () => {
        const state = get();
        const generation = statusGeneration;
        let agentRunning = false;
        let agentReplaced = false;
        try {
            const agent = await window.api.discordBotStatus();
            agentRunning = agent === 'running';
            agentReplaced = agent === 'replaced';
        } catch {
            /* stopped */
        }

        let relayOk = false;
        if (state.discordCloudHostId) {
            const relayUrl = state.discordCloudRelayUrl || RELAY_URL_FALLBACK;
            try {
                // 이미 붙어 있으면 '연결 중'으로 깜빡이지 않는다 (5초마다 확인)
                if (generation === statusGeneration && !state.relayConnected) set({ relayConnecting: true });
                const result = await window.api.relayCheckHostStatus(state.discordCloudHostId, relayUrl);
                relayOk = !!result && !result.error;
            } catch {
                /* disconnected */
            }
        }
        // 그 사이 대상이 바뀌었거나 폴링이 멈췄으면 이 결과는 버린다
        if (generation !== statusGeneration) return;

        set({
            relayConnecting: false,
            relayConnected: relayOk,
            discordAgentReplaced: agentReplaced,
            discordBotStatus: agentRunning && relayOk ? 'running' : agentRunning ? 'connecting' : 'stopped',
        });
    },

    startStatusPolling: () => {
        if (get()._statusInterval) return;

        // 첫 확인은 잠깐 미룬다. 멈추면 취소되도록 타이머를 들고 있는다
        const timeout = setTimeout(async () => {
            set({ _statusTimeout: null });
            const generation = statusGeneration;
            await get().checkStatus();
            if (generation === statusGeneration) set({ botStatusReady: true });
        }, 200);
        const interval = setInterval(() => get().checkStatus(), 5000);
        set({ _statusInterval: interval, _statusTimeout: timeout });
    },

    stopStatusPolling: () => {
        const state = get();
        clearInterval(state._statusInterval);
        clearTimeout(state._statusTimeout);
        statusGeneration += 1;
        set({ _statusInterval: null, _statusTimeout: null, relayConnecting: false });
    },

    // Test-only: reset to initial state
    _resetForTest: () => {
        const state = get();
        clearInterval(state._statusInterval);
        clearTimeout(state._statusTimeout);
        statusGeneration += 1;
        synced = {};
        set(initialState());
    },
}));

// ── Auto-save subscription (Zustand v5 single-listener) ──
let botConfigSaveTimer = null;
const _discordSaveKeys = [
    'discordPrefixes',
    'discordCloudRelayUrl',
    'discordCloudHostId',
    'nodeSettings',
    'cloudNodes',
    'cloudMembers',
    'discordModuleAliases',
    'discordCommandAliases',
];
useDiscordStore.subscribe((state, prevState) => {
    if (!state._settingsReady) return;
    const changed = _discordSaveKeys.some((k) => state[k] !== prevState[k]);
    if (!changed) return;
    // _botConfigLoaded가 false면 로드 실패 상태 — 자동 저장 시 파일 값 덮어쓰기 방지
    if (!state._botConfigLoaded) {
        console.warn('[Settings] Discord config changed but bot config not loaded — skipping auto-save');
        return;
    }
    clearTimeout(botConfigSaveTimer);
    botConfigSaveTimer = setTimeout(() => {
        useDiscordStore.getState().saveConfig();
    }, 500);
});

// ── Vite HMR: preserve store state across hot module replacement ──
if (import.meta.hot) {
    import.meta.hot.dispose((data) => {
        const s = useDiscordStore.getState();
        data.prevState = {
            discordPrefixes: s.discordPrefixes,
            discordModuleAliases: s.discordModuleAliases,
            discordCommandAliases: s.discordCommandAliases,
            discordCloudRelayUrl: s.discordCloudRelayUrl,
            discordCloudHostId: s.discordCloudHostId,
            nodeSettings: s.nodeSettings,
            cloudNodes: s.cloudNodes,
            cloudMembers: s.cloudMembers,
            discordBotStatus: s.discordBotStatus,
            relayConnected: s.relayConnected,
            botStatusReady: s.botStatusReady,
            _settingsReady: s._settingsReady,
            _botConfigLoaded: s._botConfigLoaded,
        };
    });
    if (import.meta.hot.data?.prevState) {
        useDiscordStore.setState(import.meta.hot.data.prevState);
    }
}
