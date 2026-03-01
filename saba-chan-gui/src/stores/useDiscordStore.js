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
    _autoStartDone: false,
    _statusInterval: null,
    _discordTokenRef: '',

    // ── Actions ──

    setDiscordToken: (val) => {
        set({ discordToken: val, _discordTokenRef: val });
    },

    update: (partial) => set(partial),

    loadConfig: async () => {
        try {
            const botCfg = await window.api.botConfigLoad();
            if (botCfg) {
                const patch = {
                    discordPrefix: botCfg.prefix || '!saba',
                    discordModuleAliases: botCfg.moduleAliases || {},
                    discordCommandAliases: botCfg.commandAliases || {},
                    discordMusicEnabled: botCfg.musicEnabled !== false,
                    discordBotMode: botCfg.mode || 'local',
                    discordCloudRelayUrl: botCfg.cloud?.relayUrl || '',
                    discordCloudHostId: botCfg.cloud?.hostId || '',
                };

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
            }
        } catch (err) {
            console.error('Failed to load bot config:', err);
        }
    },

    saveConfig: async (newPrefix) => {
        const state = get();
        try {
            const payload = {
                prefix: newPrefix || state.discordPrefix || '!saba',
                mode: state.discordBotMode,
                cloud: {
                    relayUrl: state.discordCloudRelayUrl,
                    hostId: state.discordCloudHostId,
                },
                moduleAliases: state.discordModuleAliases,
                commandAliases: state.discordCommandAliases,
                musicEnabled: state.discordMusicEnabled,
                nodeSettings: state.nodeSettings,
                cloudNodes: state.cloudNodes,
                cloudMembers: state.cloudMembers,
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
                    const resp = await fetch(`${relayUrl}/api/hosts/${encodeURIComponent(state.discordCloudHostId)}`, {
                        signal: AbortSignal.timeout(5000),
                    });
                    relayOk = resp.ok;
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
                const msg = data.message || _t('discord_bot.unknown_error');
                safeShowToast(msg, 'error', 6000);
            };
            window.api.onBotError(handler);
        }

        // Bot relaunch listener
        if (window.api?.onBotRelaunch) {
            const handler = (botConfig) => {
                console.log('[Bot Relaunch] Received signal to relaunch bot with new language settings');
                setTimeout(async () => {
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
                }, 1000);
            };
            window.api.onBotRelaunch(handler);
        }
    },

    // Test-only: reset to initial state
    _resetForTest: () => {
        const state = get();
        if (state._statusInterval) clearInterval(state._statusInterval);
        set({
            discordToken: '',
            discordPrefix: '!saba',
            discordAutoStart: false,
            discordMusicEnabled: true,
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
            _autoStartDone: false,
            _statusInterval: null,
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

        if (state.discordBotMode === 'cloud') {
            if (state.discordCloudHostId && state.discordPrefix && state.discordBotStatus === 'stopped') {
                if (!isTest) console.log('[Auto-start] Cloud mode — starting relay agent');
                get().startBot();
            }
        } else if (
            state.discordAutoStart &&
            state.discordToken &&
            state.discordPrefix &&
            state.discordBotStatus === 'stopped'
        ) {
            if (!isTest) console.log('[Auto-start] Starting Discord bot automatically!');
            get().startBot();
        }
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
];
useDiscordStore.subscribe((state, prevState) => {
    if (!state._settingsReady) return;
    const changed = _discordSaveKeys.some((k) => state[k] !== prevState[k]);
    if (!changed) return;
    console.log('[Settings] Discord config changed, saving...');
    clearTimeout(botConfigSaveTimer);
    botConfigSaveTimer = setTimeout(() => {
        useDiscordStore.getState().saveConfig();
    }, 500);
});

// ── Cross-store sync: token/autoStart → settings store ──
useDiscordStore.subscribe((state, prevState) => {
    if (state.discordToken !== prevState.discordToken || state.discordAutoStart !== prevState.discordAutoStart) {
        useSettingsStore.getState()._setDiscordFields(state.discordToken, state.discordAutoStart);
    }
});

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
        };
    });
    if (import.meta.hot.data?.prevState) {
        useDiscordStore.setState(import.meta.hot.data.prevState);
    }
}
