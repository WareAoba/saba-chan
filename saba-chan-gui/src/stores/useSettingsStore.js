import { create } from 'zustand';
import { THEME_DEFAULTS } from '../utils/themeManager';

export const DEFAULT_IPC_PORT = 57474; // shared/constants.js와 동일 — renderer는 CJS require 불가하므로 상수 선언

export const useSettingsStore = create((set, get) => ({
    // ── State ──
    autoRefresh: true,
    refreshInterval: 2000,
    ipcPort: DEFAULT_IPC_PORT,
    consoleBufferSize: 2000,
    autoGeneratePasswords: true,
    portConflictCheck: true,
    settingsPath: '',
    settingsReady: false,

    // ── Theme customization ──
    accentColor: THEME_DEFAULTS.accentColor,
    accentSecondary: THEME_DEFAULTS.accentSecondary,
    useGradient: THEME_DEFAULTS.useGradient,
    fontScale: THEME_DEFAULTS.fontScale,
    enableTransitions: THEME_DEFAULTS.enableTransitions,
    consoleSyntaxHighlight: THEME_DEFAULTS.consoleSyntaxHighlight,
    consoleBgColor: THEME_DEFAULTS.consoleBgColor,
    consoleTextColor: THEME_DEFAULTS.consoleTextColor,
    sidebarCompact: THEME_DEFAULTS.sidebarCompact,
    consoleFontScale: THEME_DEFAULTS.consoleFontScale,

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
                patch.ipcPort = settings.ipcPort ?? DEFAULT_IPC_PORT;
                patch.consoleBufferSize = settings.consoleBufferSize ?? 2000;
                patch.autoGeneratePasswords = settings.autoGeneratePasswords ?? true;
                patch.portConflictCheck = settings.portConflictCheck ?? true;

                // Theme customization
                patch.accentColor = settings.accentColor ?? THEME_DEFAULTS.accentColor;
                patch.accentSecondary = settings.accentSecondary ?? THEME_DEFAULTS.accentSecondary;
                patch.useGradient = settings.useGradient ?? THEME_DEFAULTS.useGradient;
                patch.fontScale = settings.fontScale ?? THEME_DEFAULTS.fontScale;
                patch.enableTransitions = settings.enableTransitions ?? THEME_DEFAULTS.enableTransitions;
                patch.consoleSyntaxHighlight = settings.consoleSyntaxHighlight ?? THEME_DEFAULTS.consoleSyntaxHighlight;
                patch.consoleBgColor = settings.consoleBgColor ?? THEME_DEFAULTS.consoleBgColor;
                patch.consoleTextColor = settings.consoleTextColor ?? THEME_DEFAULTS.consoleTextColor;
                patch.sidebarCompact = settings.sidebarCompact ?? THEME_DEFAULTS.sidebarCompact;
                patch.consoleFontScale = settings.consoleFontScale ?? THEME_DEFAULTS.consoleFontScale;
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
                // Theme customization
                accentColor: state.accentColor,
                accentSecondary: state.accentSecondary,
                useGradient: state.useGradient,
                fontScale: state.fontScale,
                enableTransitions: state.enableTransitions,
                consoleSyntaxHighlight: state.consoleSyntaxHighlight,
                consoleBgColor: state.consoleBgColor,
                consoleTextColor: state.consoleTextColor,
                sidebarCompact: state.sidebarCompact,
                consoleFontScale: state.consoleFontScale,
                // discordToken/discordAutoStart는 bot-config.json으로 이전됨
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
            ipcPort: DEFAULT_IPC_PORT,
            consoleBufferSize: 2000,
            autoGeneratePasswords: true,
            portConflictCheck: true,
            settingsPath: '',
            settingsReady: false,
            _discordToken: '',
            _discordAutoStart: false,
            accentColor: THEME_DEFAULTS.accentColor,
            accentSecondary: THEME_DEFAULTS.accentSecondary,
            useGradient: THEME_DEFAULTS.useGradient,
            fontScale: THEME_DEFAULTS.fontScale,
            enableTransitions: THEME_DEFAULTS.enableTransitions,
            consoleSyntaxHighlight: THEME_DEFAULTS.consoleSyntaxHighlight,
            consoleBgColor: THEME_DEFAULTS.consoleBgColor,
            consoleTextColor: THEME_DEFAULTS.consoleTextColor,
            sidebarCompact: THEME_DEFAULTS.sidebarCompact,
            consoleFontScale: THEME_DEFAULTS.consoleFontScale,
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
    // Theme customization
    'accentColor',
    'accentSecondary',
    'useGradient',
    'fontScale',
    'enableTransitions',
    'consoleSyntaxHighlight',
    'consoleBgColor',
    'consoleTextColor',
    'sidebarCompact',
    'consoleFontScale',
    // _discordToken, _discordAutoStart는 bot-config.json으로 이전됨 (v0.2+)
    // 하위 호환: save() 시에는 포함하지 않지만, load() 시 마이그레이션용으로 읽음
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
            accentColor: s.accentColor,
            accentSecondary: s.accentSecondary,
            useGradient: s.useGradient,
            fontScale: s.fontScale,
            enableTransitions: s.enableTransitions,
            consoleSyntaxHighlight: s.consoleSyntaxHighlight,
            consoleBgColor: s.consoleBgColor,
            consoleTextColor: s.consoleTextColor,
            sidebarCompact: s.sidebarCompact,
            consoleFontScale: s.consoleFontScale,
        };
    });
    if (import.meta.hot.data?.prevState) {
        useSettingsStore.setState(import.meta.hot.data.prevState);
    }
}
