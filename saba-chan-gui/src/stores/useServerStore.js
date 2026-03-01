import { create } from 'zustand';
import { createTranslateError, debugWarn, retryWithBackoff, safeShowToast, waitForDaemon } from '../utils/helpers';
import { useUIStore } from './useUIStore';

// i18n translate function — set after store creation
let _t = (key, fallback) => fallback || key;
let _translateError = (msg) => msg;
let _i18n = null;

export const setServerI18n = (t, i18n) => {
    _t = t;
    _translateError = createTranslateError(t);
    _i18n = i18n;
};

export const useServerStore = create((set, get) => ({
    // ── State ──
    servers: [],
    modules: [],
    loading: true,
    moduleAliasesPerModule: {},

    // ── Init state ──
    daemonReady: false,
    initStatus: 'Initialize...',
    initProgress: 0,
    serversInitializing: true,

    // ── Uptime ──
    nowEpoch: Math.floor(Date.now() / 1000),
    _uptimeInterval: null,

    // ── Internal refs ──
    _guiInitiatedOps: new Set(),
    _optimisticStatus: new Map(),
    _lastErrorToast: 0,
    _firstFetchDone: false,
    _openSettingsToExtensions: null,

    // Test-only: reset to initial state
    _resetForTest: () => {
        const state = get();
        if (state._uptimeInterval) clearInterval(state._uptimeInterval);
        set({
            servers: [],
            modules: [],
            loading: true,
            moduleAliasesPerModule: {},
            daemonReady: false,
            initStatus: 'Initialize...',
            initProgress: 0,
            serversInitializing: true,
            nowEpoch: Math.floor(Date.now() / 1000),
            _uptimeInterval: null,
            _guiInitiatedOps: new Set(),
            _optimisticStatus: new Map(),
            _lastErrorToast: 0,
            _firstFetchDone: false,
            _openSettingsToExtensions: null,
        });
    },

    // ── Actions ──

    setServers: (serversOrUpdater) => {
        if (typeof serversOrUpdater === 'function') {
            set((state) => ({ servers: serversOrUpdater(state.servers) }));
        } else {
            set({ servers: serversOrUpdater });
        }
    },

    setModules: (modules) => set({ modules }),

    setDaemonReady: (val) => set({ daemonReady: val }),
    setInitStatus: (val) => set({ initStatus: val }),
    setInitProgress: (val) => {
        if (typeof val === 'function') {
            set((state) => ({ initProgress: val(state.initProgress) }));
        } else {
            set({ initProgress: val });
        }
    },
    setServersInitializing: (val) => set({ serversInitializing: val }),

    startUptimeClock: () => {
        const state = get();
        if (state._uptimeInterval) return;
        const interval = setInterval(() => {
            set({ nowEpoch: Math.floor(Date.now() / 1000) });
        }, 1000);
        set({ _uptimeInterval: interval });
    },

    stopUptimeClock: () => {
        const state = get();
        if (state._uptimeInterval) {
            clearInterval(state._uptimeInterval);
            set({ _uptimeInterval: null });
        }
    },

    formatUptime: (startTime) => {
        if (!startTime) return null;
        const nowEpoch = get().nowEpoch;
        const elapsed = Math.max(0, nowEpoch - startTime);
        const h = String(Math.floor(elapsed / 3600)).padStart(2, '0');
        const m = String(Math.floor((elapsed % 3600) / 60)).padStart(2, '0');
        const s = String(elapsed % 60).padStart(2, '0');
        return `${h}:${m}:${s}`;
    },

    // fetchServers is managed by useServerActions hook (single source of truth)
    // to avoid duplicating change-detection + optimistic status logic.

    fetchModules: async () => {
        try {
            console.log('Fetching modules...');
            try {
                await waitForDaemon(5000);
            } catch (err) {
                debugWarn('Daemon not ready, but continuing:', err.message);
            }

            const data = await retryWithBackoff(() => window.api.moduleList(), 3, 800);
            console.log('Module data received:', data);

            if (data && data.modules) {
                console.log('Setting modules:', data.modules.length, 'modules');
                set({ modules: data.modules });

                // Register module locales
                if (_i18n) {
                    for (const mod of data.modules) {
                        try {
                            if (window.api.moduleGetLocales) {
                                const locales = await window.api.moduleGetLocales(mod.name);
                                if (locales && typeof locales === 'object') {
                                    for (const [lang, localeData] of Object.entries(locales)) {
                                        _i18n.addResourceBundle(lang, `mod_${mod.name}`, localeData, true, true);
                                    }
                                    console.log(`Module locales registered for ${mod.name}:`, Object.keys(locales));
                                }
                            }
                        } catch (e) {
                            console.warn(`Failed to load locales for module ${mod.name}:`, e);
                        }
                    }
                }

                // Load module metadata (aliases)
                const aliasesMap = {};
                for (const mod of data.modules) {
                    try {
                        const metadata = await window.api.moduleGetMetadata(mod.name);
                        if (metadata && metadata.toml) {
                            const aliases = metadata.toml.aliases || {};
                            const aliasCommands = aliases.commands || {};
                            const commandFields = metadata.toml.commands?.fields || [];
                            const mergedCommands = {};

                            for (const [cmdName, cmdData] of Object.entries(aliasCommands)) {
                                mergedCommands[cmdName] = {
                                    aliases: cmdData.aliases || [],
                                    description: cmdData.description || '',
                                    label: cmdName,
                                };
                            }
                            for (const cmdField of commandFields) {
                                const cmdName = cmdField.name;
                                if (!mergedCommands[cmdName]) {
                                    mergedCommands[cmdName] = {
                                        aliases: [],
                                        description: cmdField.description || '',
                                        label: cmdField.label || cmdName,
                                    };
                                } else {
                                    if (!mergedCommands[cmdName].description && cmdField.description)
                                        mergedCommands[cmdName].description = cmdField.description;
                                    if (cmdField.label) mergedCommands[cmdName].label = cmdField.label;
                                }
                            }
                            aliasesMap[mod.name] = { ...aliases, commands: mergedCommands };
                        }
                    } catch (e) {
                        console.warn(`Failed to load metadata for module ${mod.name}:`, e);
                    }
                }
                set({ moduleAliasesPerModule: aliasesMap });
                console.log('Module aliases loaded:', aliasesMap);
            } else if (data && data.error) {
                console.error('Module fetch error:', data.error);
                safeShowToast(_t('modules.load_failed_toast', { error: _translateError(data.error) }), 'error', 4000);
            } else {
                debugWarn('No modules data:', data);
                safeShowToast(_t('modules.list_empty'), 'warning', 3000);
            }
        } catch (error) {
            console.error('Failed to fetch modules:', error);
            safeShowToast(_t('modules.fetch_failed_toast', { error: _translateError(error.message) }), 'error', 5000);
            useUIStore.getState().openModal({
                type: 'failure',
                title: _t('modules.load_error_title'),
                message: _translateError(error.message),
            });
        }
    },
}));
// ── Vite HMR: preserve store state across hot module replacement ──
if (import.meta.hot) {
    import.meta.hot.dispose((data) => {
        const s = useServerStore.getState();
        data.prevState = {
            servers: s.servers,
            modules: s.modules,
            loading: s.loading,
            moduleAliasesPerModule: s.moduleAliasesPerModule,
            daemonReady: s.daemonReady,
            initStatus: s.initStatus,
            initProgress: s.initProgress,
            serversInitializing: s.serversInitializing,
            nowEpoch: s.nowEpoch,
            _firstFetchDone: s._firstFetchDone,
        };
    });
    if (import.meta.hot.data?.prevState) {
        useServerStore.setState(import.meta.hot.data.prevState);
    }
}
