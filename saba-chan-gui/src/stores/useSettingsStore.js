import { create } from 'zustand';
import { THEME_DEFAULTS, clearStoredTheme, getTheme, resetCustomTheme, setCustomTheme, setTheme } from '../utils/themeManager';

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
    // 테마 모드(auto/light/dark)도 config.json이 원천이다. localStorage는 첫 화면 깜빡임 방지용 캐시.
    theme: 'auto',
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
    // 서버 페이지의 시스템 모니터를 펼쳐 둘지 (접으면 요약 한 줄)
    systemMonitorOpen: true,

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
                patch.theme = settings.theme ?? getTheme();
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
                patch.systemMonitorOpen = settings.systemMonitorOpen ?? true;
            }
            const path = await window.api.settingsGetPath();
            patch.settingsPath = path;
            if (!isTest) console.log('[Settings] GUI settings loaded from:', path);

            patch.settingsReady = true;
            set(patch);
            if (settings) {
                // config.json 값을 화면에 적용하고 캐시도 맞춘다
                setTheme(patch.theme);
                setCustomTheme({
                    accentColor: patch.accentColor,
                    accentSecondary: patch.accentSecondary,
                    useGradient: patch.useGradient,
                    fontScale: patch.fontScale,
                    enableTransitions: patch.enableTransitions,
                    consoleSyntaxHighlight: patch.consoleSyntaxHighlight,
                    consoleBgColor: patch.consoleBgColor,
                    consoleTextColor: patch.consoleTextColor,
                    consoleFontScale: patch.consoleFontScale,
                    sidebarCompact: patch.sidebarCompact,
                });
                // 예전 버전은 테마 모드를 브라우저 저장소에만 뒀다 — 한 번 config.json으로 옮긴다
                if (settings.theme == null && !isTest) get().save();
            }
            if (!isTest) console.log('[Settings] Ready flag set to true');
            return settings; // Return raw settings for discord token etc.
        } catch (error) {
            console.error('[Settings] Failed to load settings:', error);
            set({ settingsReady: true });
            return null;
        }
    },

    /** GUI 설정을 데몬 기본값으로 되돌리고 다시 읽는다 (창 위치·언어는 데몬이 정한 대로) */
    resetToDefaults: async () => {
        if (!window.api?.configReset) return false;
        try {
            const res = await window.api.configReset('gui');
            if (res?.ok === false) return false;
            // 데몬 기본값에는 theme이 없다 → 브라우저 캐시를 지워야 load()가 예전 값 대신 'auto'를 쓴다
            clearStoredTheme();
            resetCustomTheme();
            await get().load();
            return true;
        } catch (error) {
            console.error('[Settings] Failed to reset settings:', error);
            return false;
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
                theme: state.theme,
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
                systemMonitorOpen: state.systemMonitorOpen,
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
            theme: 'auto',
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
            systemMonitorOpen: true,
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
    'theme',
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
    'systemMonitorOpen',
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
            theme: s.theme,
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
