import { create } from 'zustand';
import { createTranslateError, safeShowToast } from '../utils/helpers';
import { useSettingsStore } from './useSettingsStore';
import { useUIStore } from './useUIStore';

// i18n translate function — set after store creation
let _t = (key, fallback) => fallback || key;
let _translateError = (msg) => msg;

export const setDiscordI18n = (t) => {
    _t = t;
    _translateError = createTranslateError(t);
};

const RELAY_URL_FALLBACK = 'https://saba-chan.online';

export const useDiscordStore = create((set, get) => ({
    // ── Bot config ──
    discordToken: '',
    discordPrefix: '!saba',
    discordAutoStart: false,
    discordMusicEnabled: true,
    discordMusicChannelId: '',
    discordMusicUISettings: { queueLines: 5, refreshInterval: 4000 },
    discordModuleAliases: {},
    discordCommandAliases: {},

    // ── Bot mode ──
    discordBotMode: 'local',
    discordCloudRelayUrl: '',
    discordCloudHostId: '',

    // ── Node & cloud ──
    nodeSettings: {},
    cloudNodes: [],
    cloudMembers: {},

    // ── Runtime status ──
    discordBotStatus: 'stopped',
    botStatusReady: false,
    relayConnected: false,
    relayConnecting: false,

    // ── Internal ──
    _settingsReady: false,
    _botConfigLoaded: false,
    _autoStartDone: false,
    _statusInterval: null,
    _autoStartTimer: null,
    _autoStartRetryTimer: null,
    _discordTokenRef: '',

    // ── Actions ──

    setDiscordToken: (val) => {
        set({ discordToken: val, _discordTokenRef: val });
    },

    update: (partial) => set(partial),

    // ── 모드 전환 (디바운스 + 자동 재시작) ──
    _modeSwitchTimer: null,
    switchMode: (newMode) => {
        const state = get();
        const prevMode = state.discordBotMode;
        if (newMode === prevMode) return;

        // 즉시 모드 반영 (UI 업데이트)
        set({ discordBotMode: newMode });

        // 기존 디바운스 타이머 취소
        if (state._modeSwitchTimer) {
            clearTimeout(state._modeSwitchTimer);
        }

        // 디바운스: 빠른 토글 시 마지막 전환만 실행
        const timer = setTimeout(async () => {
            const s = get();

            // 1. 실행 중인 봇 정지
            if (s.discordBotStatus === 'running' || s.discordBotStatus === 'connecting') {
                try {
                    await window.api.discordBotStop();
                    set({ discordBotStatus: 'stopped' });
                } catch (_) { /* ignore */ }
                // 프로세스 정리 대기
                await new Promise((r) => setTimeout(r, 500));
            }

            // 2. 새 모드로 봇 자동 시작 (조건 충족 시)
            const cur = get();
            const isCloud = cur.discordBotMode === 'cloud';
            const canStart = isCloud
                ? !!(cur.discordCloudHostId && cur.discordPrefix)
                : !!(cur.discordToken && cur.discordPrefix);

            if (canStart) {
                await cur.startBot();
            }
        }, 800);

        set({ _modeSwitchTimer: timer });
    },

    loadConfig: async () => {
        const isTest = process.env.NODE_ENV === 'test' || typeof jest !== 'undefined';
        const maxAttempts = isTest ? 1 : 5;
        const retryDelay = 800;

        for (let attempt = 1; attempt <= maxAttempts; attempt++) {
            try {
                const botCfg = await window.api.botConfigLoad();
                if (!botCfg) continue;

                // 데몬이 파일을 읽지 못한 경우 기본값(prefix, moduleAliases, commandAliases만 존재)을 반환함
                // 이 경우 musicChannelId 등이 누락되어 스토어를 덮어쓰면 기존 설정이 손실됨
                const isFullConfig = 'musicChannelId' in botCfg || 'mode' in botCfg || 'musicEnabled' in botCfg;

                if (!isFullConfig && attempt < maxAttempts) {
                    if (!isTest) console.log(`[Settings] Got partial bot config, retrying (${attempt}/${maxAttempts})...`);
                    await new Promise((r) => setTimeout(r, retryDelay));
                    continue;
                }

                const patch = {
                    discordPrefix: botCfg.prefix || '!saba',
                    discordModuleAliases: botCfg.moduleAliases || {},
                    discordCommandAliases: botCfg.commandAliases || {},
                    discordMusicEnabled: botCfg.musicEnabled !== false,
                    discordMusicChannelId: botCfg.musicChannelId ?? '',
                    discordMusicUISettings: botCfg.musicUISettings || { queueLines: 5, refreshInterval: 4000, normalize: true },
                    discordBotMode: botCfg.mode || 'local',
                    discordCloudRelayUrl: botCfg.cloud?.relayUrl || '',
                    discordCloudHostId: botCfg.cloud?.hostId || '',
                    _botConfigLoaded: isFullConfig,
                };

                // token과 autoStart는 bot-config.json이 SSOT
                if ('token' in botCfg) {
                    patch.discordToken = botCfg.token || '';
                    patch._discordTokenRef = botCfg.token || '';
                }
                if ('autoStart' in botCfg) {
                    patch.discordAutoStart = botCfg.autoStart ?? false;
                }

                if (botCfg.nodeSettings && typeof botCfg.nodeSettings === 'object') {
                    patch.nodeSettings = botCfg.nodeSettings;
                } else if (Array.isArray(botCfg.allowedInstances)) {
                    patch.nodeSettings = {
                        local: { allowedInstances: botCfg.allowedInstances, memberPermissions: {} },
                    };
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
            // 기존 파일 내용을 읽어서 병합 — 로드 실패 시 설정 손실 방지
            let base = {};
            if (!state._botConfigLoaded) {
                try {
                    const current = await window.api.botConfigLoad();
                    if (current) base = current;
                } catch (_) { /* 데몬 미응답 — 빈 base 사용 */ }
            }

            const payload = {
                ...base,
                prefix: newPrefix || state.discordPrefix || '!saba',
                token: state.discordToken || base.token || '',
                autoStart: state.discordAutoStart ?? base.autoStart ?? false,
                mode: state.discordBotMode || base.mode || 'local',
                cloud: {
                    relayUrl: state.discordCloudRelayUrl ?? base.cloud?.relayUrl ?? '',
                    hostId: state.discordCloudHostId ?? base.cloud?.hostId ?? '',
                },
                moduleAliases: state.discordModuleAliases ?? base.moduleAliases ?? {},
                commandAliases: state.discordCommandAliases ?? base.commandAliases ?? {},
                musicEnabled: state.discordMusicEnabled,
                // _botConfigLoaded가 true면 스토어 값이 정확 → 직접 사용
                // false면 로드 실패로 스토어가 빈값일 수 있음 → 파일 값 우선
                musicChannelId: state._botConfigLoaded
                    ? state.discordMusicChannelId
                    : (state.discordMusicChannelId || base.musicChannelId || ''),
                musicUISettings: state.discordMusicUISettings ?? base.musicUISettings ?? { queueLines: 5, refreshInterval: 4000, normalize: true },
                nodeSettings: state.nodeSettings ?? base.nodeSettings ?? {},
                cloudNodes: state.cloudNodes ?? base.cloudNodes ?? [],
                cloudMembers: state.cloudMembers ?? base.cloudMembers ?? {},
            };
            const res = await window.api.botConfigSave(payload);
            if (res.error) {
                console.error('[Settings] Failed to save bot config:', res.error);
                safeShowToast(_t('settings.save_error'), 'error');
            } else {
                console.log('[Settings] Bot config saved, prefix:', payload.prefix);
            }
        } catch (error) {
            console.error('[Settings] Failed to save bot config:', error);
                safeShowToast(_t('settings.save_error'), 'error');
        }
    },

    startBot: async () => {
        const state = get();
        const isCloud = state.discordBotMode === 'cloud';

        if (isCloud && !state.discordCloudHostId) {
            return;
        }
        if (!isCloud && !state.discordToken) {
            useUIStore.getState().openModal({
                type: 'failure',
                title: _t('discord_bot.token_missing_title'),
                message: _t('discord_bot.token_missing_message'),
            });
            return;
        }
        if (!state.discordPrefix) {
            useUIStore.getState().openModal({
                type: 'failure',
                title: _t('discord_bot.prefix_missing_title'),
                message: _t('discord_bot.prefix_missing_message'),
            });
            return;
        }
        try {
            const botConfig = {
                token: state.discordToken,
                prefix: state.discordPrefix,
                moduleAliases: state.discordModuleAliases,
                commandAliases: state.discordCommandAliases,
                musicEnabled: state.discordMusicEnabled,
                musicChannelId: state.discordMusicChannelId,
                musicUISettings: state.discordMusicUISettings,
                mode: state.discordBotMode || 'local',
                cloud: {
                    relayUrl: state.discordCloudRelayUrl || '',
                    hostId: state.discordCloudHostId || '',
                },
                nodeSettings: state.nodeSettings || {},
            };
            const result = await window.api.discordBotStart(botConfig);
            if (result.error) {
                safeShowToast(
                    _t('discord_bot.start_failed_toast', { error: _translateError(result.error) }),
                    'error',
                    4000,
                );
            } else {
                set({ discordBotStatus: 'running' });
                safeShowToast(_t('discord_bot.started_toast'), 'discord', 3000);
            }
        } catch (e) {
            safeShowToast(_t('discord_bot.start_error_toast', { error: _translateError(e.message) }), 'error', 4000);
        }
    },

    stopBot: async () => {
        try {
            const result = await window.api.discordBotStop();
            if (result.error) {
                safeShowToast(
                    _t('discord_bot.stop_failed_toast', { error: _translateError(result.error) }),
                    'error',
                    4000,
                );
            } else {
                set({ discordBotStatus: 'stopped' });
                safeShowToast(_t('discord_bot.stopped_toast'), 'discord', 3000);
            }
        } catch (e) {
            safeShowToast(_t('discord_bot.stop_error_toast', { error: _translateError(e.message) }), 'error', 4000);
        }
    },

    checkStatus: async () => {
        const state = get();
        if (state.discordBotMode === 'cloud') {
            let agentRunning = false;
            try {
                const status = await window.api.discordBotStatus();
                agentRunning = status === 'running';
            } catch {
                /* stopped */
            }

            let relayOk = false;
            if (state.discordCloudHostId) {
                const relayUrl = state.discordCloudRelayUrl || RELAY_URL_FALLBACK;
                try {
                    set({ relayConnecting: true });
                    const result = await window.api.relayCheckHostStatus(state.discordCloudHostId, relayUrl);
                    relayOk = result && !result.error;
                } catch {
                    /* disconnected */
                }
                set({ relayConnecting: false });
            }

            set({
                relayConnected: relayOk,
                discordBotStatus: agentRunning && relayOk ? 'running' : agentRunning ? 'connecting' : 'stopped',
            });
        } else {
            try {
                const status = await window.api.discordBotStatus();
                set({ discordBotStatus: status === 'running' ? 'running' : 'stopped' });
            } catch {
                set({ discordBotStatus: 'stopped' });
            }
        }
    },

    startStatusPolling: () => {
        const state = get();
        if (state._statusInterval) return;

        // Initial check
        const init = async () => {
            await new Promise((resolve) => setTimeout(resolve, 200));
            await get().checkStatus();
            set({ botStatusReady: true });
            console.log('[Init] BotStatusReady flag set to true, mode:', get().discordBotMode);
        };
        init();

        const interval = setInterval(() => get().checkStatus(), 5000);
        set({ _statusInterval: interval });
    },

    stopStatusPolling: () => {
        const state = get();
        if (state._statusInterval) {
            clearInterval(state._statusInterval);
            set({ _statusInterval: null });
        }
    },

    // Initialize listeners (call once in App mount)
    initListeners: () => {
        // Bot error listener
        if (window.api?.onBotError) {
            const handler = (data) => {
                console.error('[Bot Error Event]', data);
                if (data.type === 'exit' || data.type === 'spawn_error') {
                    set({ discordBotStatus: 'stopped' });
                }

                // 정상 종료(code 0) 또는 사용자 중지 시에는 토스트 표시 안 함
                if (data.type === 'exit' && (!data.code || data.code === 0)) return;

                const msg = data.message || '';
                // 빈 메시지 또는 의미 없는 exit 코드만 있는 메시지는 무시
                if (!msg || !msg.trim()) return;

                safeShowToast(_translateError(msg), 'error', 6000);
            };
            window.api.onBotError(handler);
        }

        // Bot relaunch listener
        if (window.api?.onBotRelaunch) {
            const handler = (botConfig) => {
                console.log('[Bot Relaunch] Received signal to relaunch bot with new language settings');
                setTimeout(async () => {
                    try {
                        const state = get();
                        const configWithToken = { ...botConfig, token: state._discordTokenRef || state.discordToken };
                        const result = await window.api.discordBotStart(configWithToken);
                        if (result.error) {
                            console.error('[Bot Relaunch] Failed to relaunch bot:', result.error);
                        } else {
                            console.log('[Bot Relaunch] Bot relaunched successfully');
                            set({ discordBotStatus: 'running' });
                            safeShowToast(_t('discord_bot.relaunched_toast'), 'discord', 3000);
                        }
                    } catch (err) {
                        console.error('[Bot Relaunch] Unhandled error:', err.message);
                    }
                }, 1000);
            };
            window.api.onBotRelaunch(handler);
        }
    },

    // Test-only: reset to initial state
    _resetForTest: () => {
        const state = get();
        if (state._statusInterval) clearInterval(state._statusInterval);
        if (state._modeSwitchTimer) clearTimeout(state._modeSwitchTimer);
        if (state._autoStartTimer) clearTimeout(state._autoStartTimer);
        if (state._autoStartRetryTimer) clearTimeout(state._autoStartRetryTimer);
        set({
            discordToken: '',
            discordPrefix: '!saba',
            discordAutoStart: false,
            discordMusicEnabled: true,
            discordMusicChannelId: '',
            discordMusicUISettings: { queueLines: 5, refreshInterval: 4000, normalize: true },
            discordModuleAliases: {},
            discordCommandAliases: {},
            discordBotMode: 'local',
            discordCloudRelayUrl: '',
            discordCloudHostId: '',
            nodeSettings: {},
            cloudNodes: [],
            cloudMembers: {},
            discordBotStatus: 'stopped',
            botStatusReady: false,
            relayConnected: false,
            relayConnecting: false,
            _settingsReady: false,
            _botConfigLoaded: false,
            _autoStartDone: false,
            _statusInterval: null,
            _modeSwitchTimer: null,
            _autoStartTimer: null,
            _autoStartRetryTimer: null,
            _discordTokenRef: '',
        });
    },

    // Auto-start check (call after settings and status ready)
    tryAutoStart: () => {
        const state = get();
        if (state._autoStartDone) return;
        if (!state.botStatusReady || !state._settingsReady) return;

        set({ _autoStartDone: true });
        const isTest = process.env.NODE_ENV === 'test' || typeof jest !== 'undefined';

        const shouldStart = state.discordBotMode === 'cloud'
            ? (state.discordCloudHostId && state.discordPrefix && state.discordBotStatus === 'stopped')
            : (state.discordAutoStart && state.discordToken && state.discordPrefix && state.discordBotStatus === 'stopped');

        if (!shouldStart) return;

        if (!isTest) console.log('[Auto-start] Starting Discord bot automatically...');
        // 약간 지연 — 데몬 ext-process API 초기화 대기
        const timerId = setTimeout(async () => {
            const cur = get();
            if (cur.discordBotStatus !== 'stopped') return; // 이미 시작됨
            await cur.startBot();
            // startBot()은 내부에서 에러를 catch하므로 throw하지 않음
            // 3초 후 상태 확인하여 실패 시 재시도
            const retryId = setTimeout(async () => {
                await get().checkStatus();
                const retry = get();
                if (retry.discordBotStatus === 'stopped') {
                    if (!isTest) console.warn('[Auto-start] First attempt failed, retrying...');
                    await retry.startBot();
                }
            }, 3000);
            set({ _autoStartRetryTimer: retryId });
        }, 1500);
        set({ _autoStartTimer: timerId });
    },
}));

// ── Auto-save subscription (Zustand v5 single-listener) ──
let botConfigSaveTimer = null;
const _discordSaveKeys = [
    'discordPrefix',
    'discordBotMode',
    'discordCloudRelayUrl',
    'discordCloudHostId',
    'nodeSettings',
    'cloudNodes',
    'cloudMembers',
    'discordModuleAliases',
    'discordCommandAliases',
    'discordMusicEnabled',
    'discordMusicChannelId',
    'discordMusicUISettings',
    'discordToken',
    'discordAutoStart',
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
    console.log('[Settings] Discord config changed, saving...');
    clearTimeout(botConfigSaveTimer);
    botConfigSaveTimer = setTimeout(() => {
        useDiscordStore.getState().saveConfig();
    }, 500);
});

// ── Cross-store sync: token/autoStart 변경 시 bot-config 저장 (이제 settings store 대신 bot-config이 SSOT) ──
// token/autoStart는 _discordSaveKeys에 포함되어 위 auto-save subscription에서 처리됨

// ── Vite HMR: preserve store state across hot module replacement ──
if (import.meta.hot) {
    import.meta.hot.dispose((data) => {
        const s = useDiscordStore.getState();
        data.prevState = {
            discordToken: s.discordToken,
            discordPrefix: s.discordPrefix,
            discordAutoStart: s.discordAutoStart,
            discordModuleAliases: s.discordModuleAliases,
            discordCommandAliases: s.discordCommandAliases,
            discordMusicEnabled: s.discordMusicEnabled,
            discordMusicChannelId: s.discordMusicChannelId,
            discordMusicUISettings: s.discordMusicUISettings,
            discordBotMode: s.discordBotMode,
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
