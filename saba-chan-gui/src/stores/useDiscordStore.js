import { create } from 'zustand';
import { createTranslateError, safeShowToast } from '../utils/helpers';
import { useUIStore } from './useUIStore';

// i18n translate function — set after store creation
let _t = (key, fallback) => fallback || key;
let _translateError = (msg) => msg;

export const setDiscordI18n = (t) => {
    _t = t;
    _translateError = createTranslateError(t);
};

const RELAY_URL_FALLBACK = 'https://saba-chan.online';
// 상태 확인 세대: 연결 대상이 바뀌거나 폴링이 멈추면 올려서, 늦게 온 옛 응답을 버린다
let statusGeneration = 0;

// 디스코드 봇 본체는 클라우드(릴레이 서버)에서 돌고, 로컬에서는 데몬 내장
// 릴레이 에이전트가 명령을 받아 처리한다. 이 스토어는 bot-config와
// 에이전트/릴레이 상태만 다룬다. 에이전트는 노드 토큰이 있으면 데몬이 자동 시작한다.
const initialState = () => ({
    // ── Bot config (bot-config.json, 데몬이 SSOT) ──
    discordPrefix: '!saba',
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
                    discordPrefix: botCfg.prefix || '!saba',
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

    saveConfig: async (newPrefix) => {
        const state = get();
        try {
            // 처음 로드에 실패했다면 스토어 값은 기본값이다. 그대로 보내면 디스크의
            // hostId·별명·노드 설정을 지우므로, 데몬에서 다시 읽어 빈 값은 디스크 값으로 채운다.
            // 그것도 못 읽으면 저장하지 않는다.
            let base = null;
            if (!state._botConfigLoaded) {
                base = await window.api.botConfigLoad();
                if (!base) throw new Error('bot config unavailable; refusing to overwrite it with defaults');
            }
            const isEmpty = (v) => v == null || v === '' || (typeof v === 'object' && Object.keys(v).length === 0);
            const pick = (value, baseValue, fallback) =>
                base && isEmpty(value) ? (baseValue ?? fallback) : (value ?? fallback);

            // 데몬은 최상위 키 단위로 병합하므로 이 스토어가 아는 키만 보낸다.
            const payload = {
                prefix:
                    newPrefix ||
                    (base && state.discordPrefix === '!saba' && base.prefix) ||
                    state.discordPrefix ||
                    '!saba',
                cloud: {
                    relayUrl: pick(state.discordCloudRelayUrl, base?.cloud?.relayUrl, ''),
                    hostId: pick(state.discordCloudHostId, base?.cloud?.hostId, ''),
                },
                moduleAliases: pick(state.discordModuleAliases, base?.moduleAliases, {}),
                commandAliases: pick(state.discordCommandAliases, base?.commandAliases, {}),
                nodeSettings: pick(state.nodeSettings, base?.nodeSettings, {}),
                cloudNodes: pick(state.cloudNodes, base?.cloudNodes, []),
                cloudMembers: pick(state.cloudMembers, base?.cloudMembers, {}),
            };
            const res = await window.api.botConfigSave(payload);
            if (res?.error) {
                console.error('[Settings] Failed to save bot config:', res.error);
                safeShowToast(_t('settings.save_error'), 'error');
                return false;
            }
            console.log('[Settings] Bot config saved, prefix:', payload.prefix);
            return true;
        } catch (error) {
            console.error('[Settings] Failed to save bot config:', error);
            safeShowToast(_t('settings.save_error'), 'error');
            return false;
        }
    },

    startBot: async () => {
        const state = get();
        if (!state.discordPrefix) {
            useUIStore.getState().openModal({
                type: 'failure',
                title: _t('discord_bot.prefix_missing_title'),
                message: _t('discord_bot.prefix_missing_message'),
            });
            return;
        }
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
        try {
            agentRunning = (await window.api.discordBotStatus()) === 'running';
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
        set(initialState());
    },
}));

// ── Auto-save subscription (Zustand v5 single-listener) ──
let botConfigSaveTimer = null;
const _discordSaveKeys = [
    'discordPrefix',
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
            discordPrefix: s.discordPrefix,
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
