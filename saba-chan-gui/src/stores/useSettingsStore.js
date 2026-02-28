import { create } from 'zustand';

export const useSettingsStore = create((set, get) => ({
    // ── State ──
    autoRefresh: true,
    refreshInterval: 2000,
    ipcPort: 57474,
    consoleBufferSize: 2000,
    autoGeneratePasswords: true,
    portConflictCheck: true,
    settingsPath: '',
    settingsReady: false,

    // ── Actions ──
    load: async () => {
        try {
            const isTest = process.env.NODE_ENV === 'test' || typeof jest !== 'undefined';

            const settings = await window.api.settingsLoad();
            if (!isTest) console.log('[Settings] Loaded:', settings);
            const patch = {};
            if (settings) {
                patch.autoRefresh = settings.autoRefresh ?? true;
                patch.refreshInterval = settings.refreshInterval ?? 2000;
                patch.ipcPort = settings.ipcPort ?? 57474;
                patch.consoleBufferSize = settings.consoleBufferSize ?? 2000;
                patch.autoGeneratePasswords = settings.autoGeneratePasswords ?? true;
                patch.portConflictCheck = settings.portConflictCheck ?? true;
            }
            const path = await window.api.settingsGetPath();
            patch.settingsPath = path;
            if (!isTest) console.log('[Settings] GUI settings loaded from:', path);

            patch.settingsReady = true;
            set(patch);
            if (!isTest) console.log('[Settings] Ready flag set to true');
            return settings; // Return raw settings for discord token etc.
        } catch (error) {
            console.error('[Settings] Failed to load settings:', error);
            set({ settingsReady: true });
            return null;
        }
    },

    save: async () => {
        const state = get();
        if (!state.settingsPath) {
            console.warn('[Settings] Settings path not initialized, skipping save');
            return;
        }
        try {
            await window.api.settingsSave({
                autoRefresh: state.autoRefresh,
                refreshInterval: state.refreshInterval,
                ipcPort: state.ipcPort,
                consoleBufferSize: state.consoleBufferSize,
                autoGeneratePasswords: state.autoGeneratePasswords,
                portConflictCheck: state.portConflictCheck,
                discordToken: state._discordToken || '',
                discordAutoStart: state._discordAutoStart ?? false,
            });
            console.log('[Settings] GUI settings saved');
        } catch (error) {
            console.error('[Settings] Failed to save GUI settings:', error);
        }
    },

    update: (partial) => {
        set(partial);
        // Debounced save is handled by subscription below
    },

    // Internal: discord fields that are saved alongside GUI settings
    // These are set by useDiscordStore but saved via settings file
    _discordToken: '',
    _discordAutoStart: false,
    _setDiscordFields: (token, autoStart) => set({ _discordToken: token, _discordAutoStart: autoStart }),

    // Test-only: reset to initial state
    _resetForTest: () =>
        set({
            autoRefresh: true,
            refreshInterval: 2000,
            ipcPort: 57474,
            consoleBufferSize: 2000,
            autoGeneratePasswords: true,
            portConflictCheck: true,
            settingsPath: '',
            settingsReady: false,
            _discordToken: '',
            _discordAutoStart: false,
        }),
}));

// ── Auto-save subscription (Zustand v5 single-listener) ──
let settingsSaveTimer = null;
const _settingsKeys = [
    'autoRefresh',
    'refreshInterval',
    'ipcPort',
    'consoleBufferSize',
    'autoGeneratePasswords',
    'portConflictCheck',
    '_discordAutoStart',
];
useSettingsStore.subscribe((state, prevState) => {
    if (!state.settingsReady || !state.settingsPath) return;
    const changed = _settingsKeys.some((k) => state[k] !== prevState[k]);
    if (!changed) return;
    console.log('[Settings] Settings changed, saving...');
    clearTimeout(settingsSaveTimer);
    settingsSaveTimer = setTimeout(() => {
        useSettingsStore.getState().save();
    }, 500);
});

// ── Vite HMR: preserve store state across hot module replacement ──
if (import.meta.hot) {
    import.meta.hot.dispose((data) => {
        const s = useSettingsStore.getState();
        data.prevState = {
            autoRefresh: s.autoRefresh,
            refreshInterval: s.refreshInterval,
            ipcPort: s.ipcPort,
            consoleBufferSize: s.consoleBufferSize,
            autoGeneratePasswords: s.autoGeneratePasswords,
            portConflictCheck: s.portConflictCheck,
            settingsPath: s.settingsPath,
            settingsReady: s.settingsReady,
            _discordToken: s._discordToken,
            _discordAutoStart: s._discordAutoStart,
        };
    });
    if (import.meta.hot.data?.prevState) {
        useSettingsStore.setState(import.meta.hot.data.prevState);
    }
}
