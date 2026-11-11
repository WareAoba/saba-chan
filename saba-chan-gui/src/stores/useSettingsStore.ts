import type { TFunction } from 'i18next';
import { create } from 'zustand';
import { isMonitorFloat, type MonitorFloat } from '../components/monitor/floatPosition';
import type { ConsoleFonts, GuiSettings, SoundSettings, SystemMonitorMode } from '../types/daemon';
import { safeShowToast } from '../utils/helpers';
import {
    THEME_DEFAULTS,
    WINDOW_BACKDROP_DEFAULTS,
    type WindowBackdropConfig,
    applyWindowBackdrop,
    clearStoredTheme,
    getTheme,
    resetCustomTheme,
    setCustomTheme,
    setTheme,
} from '../utils/themeManager';
import { CONSOLE_FONT_DEFAULTS } from '../utils/consoleFonts';
import { failed } from '../utils/apiResult';

/** 시스템 모니터 표시 방식 (데몬 GuiSettings.systemMonitorMode와 같다) */
export const SYSTEM_MONITOR_MODES: readonly SystemMonitorMode[] = ['off', 'docked', 'floating'];

// i18n translate function — set after store creation
let _t = (key: string): string => key;

export const setSettingsI18n = (t: TFunction | ((key: string) => string)) => {
    _t = t as (key: string) => string;
};

// 화면에서 바꿨지만 아직 저장하지 않은 칸. 저장은 이 칸만 보낸다 (CLI가 저장한 다른 값을 옛 값으로 덮지 않게)
const dirtyKeys = new Set<SettingKey>();
// 데몬에서 읽은 값을 넣는 중: 바꾼 칸으로 치지 않는다
let applyingLoad = false;

export const DEFAULT_IPC_PORT = 57474; // updater/src/constants.rs DEFAULT_IPC_PORT와 같다

// 효과음 스위치 — 데몬 GuiSettings.sounds와 같은 모양. 전체(enabled)와 종류가 모두 켜져야 울린다
export const DEFAULT_SOUNDS: Readonly<SoundSettings> = Object.freeze({
    enabled: true,
    serverStart: true,
    serverStop: true,
    crash: true,
    install: true,
    update: true,
    startup: true,
});

// 앱 동작 옵션 (사용자 10-27). 기본값은 데몬 GuiSettings와 같다: 지금까지의 동작 그대로
export const APP_OPTION_DEFAULTS: Readonly<AppOptions> = Object.freeze({
    closeAction: 'ask', // 창 닫기: ask | tray | quit
    toastDuration: 'normal', // 알림 표시 시간: short | normal | long
    newServerBackupKeep: 10, // 새 서버의 자동 백업 보관 개수 (데몬이 만들 때 넣는다)
    newServerAutoRestart: true, // 새 서버의 크래시 자동 재시작
    auditKeepDays: 0, // 작업 기록 보관 일수, 0은 용량 한도만 (데몬이 지운다)
    consoleTimestamps: 'off', // 콘솔 줄 앞 시각: off | 24h | 12h
    consoleTimestampDate: false,
    consoleWrap: true, // 콘솔 긴 줄 접기 (끄면 가로 스크롤)
    consoleAutoScroll: true, // 맨 아래를 볼 때 새 줄 따라가기
    consoleFonts: CONSOLE_FONT_DEFAULTS, // 콘솔 글꼴: 글자마다 하나 (utils/consoleFonts.js)
    systemMonitorNetTrace: false, // 고급: 포트·서버별 네트워크 (모니터를 펼쳐 둔 동안 데몬이 센다)
});

/** 앱 동작 옵션 (APP_OPTION_DEFAULTS의 칸) */
type AppOptionKey =
    | 'closeAction'
    | 'toastDuration'
    | 'newServerBackupKeep'
    | 'newServerAutoRestart'
    | 'auditKeepDays'
    | 'consoleTimestamps'
    | 'consoleTimestampDate'
    | 'consoleWrap'
    | 'consoleAutoScroll'
    | 'consoleFonts'
    | 'systemMonitorNetTrace';
type AppOptions = Pick<GuiSettings, AppOptionKey> & { consoleFonts: Readonly<ConsoleFonts> };

/** config.json에 저장하는 칸 (_settingsKeys) */
export interface SettingsValues extends AppOptions {
    autoRefresh: boolean;
    refreshInterval: number;
    ipcPort: number;
    consoleBufferSize: number;
    autoGeneratePasswords: boolean;
    portConflictCheck: boolean;
    autoPortCheck: boolean;
    osNotifications: boolean;
    sounds: Readonly<SoundSettings>;
    theme: string;
    accentColor: string;
    accentSecondary: string;
    useGradient: boolean;
    fontScale: number;
    enableTransitions: boolean;
    consoleSyntaxHighlight: boolean;
    consoleBgColor: string;
    consoleTextColor: string;
    sidebarCompact: boolean;
    consoleFontScale: number;
    consoleTokenColors: Record<string, string>;
    /** 창 바탕 반투명 (셸이 OS 효과를 깔고 화면이 테마 바탕을 덮는다) */
    windowBackdrop: Readonly<WindowBackdropConfig>;
    systemMonitorExpanded: boolean;
    /** 시스템 모니터에 띄울 항목 id (monitor/systemMetrics.ts). null이면 기본 항목 */
    monitorItems: string[] | null;
    /** 시스템 모니터 표시 방식 (끔·고정·떠 있는 창) */
    systemMonitorMode: SystemMonitorMode;
    /** 떠 있는 시스템 모니터 자리. null이면 오른쪽 위 */
    systemMonitorFloat: MonitorFloat | null;
    /** ✕로 닫은 가이드 id (components/guides/guides.ts) */
    closedGuides: string[];
    /** switch: 고른 데몬만 · unified: 다른 데몬의 서버도 데몬별로 */
    daemonView: 'switch' | 'unified';
    /** 앱을 열면 먼저 볼 원격 데몬 hostId, null이면 이 PC */
    mainDaemon: string | null;
}
type SettingKey = keyof SettingsValues;

interface SettingsState extends SettingsValues {
    settingsPath: string;
    settingsReady: boolean;
    /** 데몬에서 읽어 화면에 넣는다. 읽은 원본을 돌려준다 (실패면 null) */
    load: () => Promise<Partial<GuiSettings> | null>;
    resetToDefaults: () => Promise<boolean>;
    /** 바뀐 칸만 저장한다. 저장할 것이 없거나 성공하면 true, 실패하면 false, 아직 준비 전이면 undefined */
    save: () => Promise<boolean | undefined>;
    update: (partial: Partial<SettingsValues>) => void;
    _resetForTest: () => void;
}

export const useSettingsStore = create<SettingsState>()((set, get) => ({
    // ── State ──
    autoRefresh: true,
    refreshInterval: 2000,
    ipcPort: DEFAULT_IPC_PORT,
    consoleBufferSize: 2000,
    autoGeneratePasswords: true,
    portConflictCheck: true,
    // 서버를 만들 때·데몬이 켜질 때 포트포워딩을 조용히 확인한다 (데몬이 한다)
    autoPortCheck: false,
    // 크래시처럼 꼭 알아야 할 일을 OS 알림으로도 (창이 앞에 있으면 앱 안 알림만)
    osNotifications: true,
    // 밖에서 켜고 끈 것·크래시·설치 결과·업데이트 발견 때 효과음 (셸이 낸다). 전체·종류별 스위치
    sounds: DEFAULT_SOUNDS,
    ...APP_OPTION_DEFAULTS,
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
    consoleTokenColors: THEME_DEFAULTS.consoleTokenColors,
    windowBackdrop: WINDOW_BACKDROP_DEFAULTS,
    // 서버 페이지의 시스템 모니터를 펼쳐 둘지 (접으면 요약 한 줄)
    systemMonitorExpanded: false,
    // 시스템 모니터에 띄울 항목 (null이면 기본)
    monitorItems: null,
    // 시스템 모니터: 끔 · 상세 위·오른쪽 열에 고정(기본) · 앱 안에서 옮기는 떠 있는 창
    systemMonitorMode: 'docked',
    systemMonitorFloat: null,
    // 닫은 가이드 (설정 → 일반에서 다시 연다)
    closedGuides: [],
    // 서버 목록 보기 (G1c): 'switch' 고른 데몬만 | 'unified' 다른 데몬의 서버도 데몬별로
    daemonView: 'switch',
    // 앱을 열면 먼저 볼 데몬: 원격 데몬 hostId, null이면 이 PC
    mainDaemon: null,

    // ── Actions ──
    load: async () => {
        try {
            const isTest = process.env.NODE_ENV === 'test' || 'jest' in globalThis;

            const settings = await window.api.settingsLoad();
            if (!isTest) console.log('[Settings] Loaded:', settings);
            const patch: Partial<SettingsState> & Record<string, unknown> = {};
            if (settings) {
                patch.autoRefresh = settings.autoRefresh ?? true;
                patch.refreshInterval = settings.refreshInterval ?? 2000;
                patch.ipcPort = settings.ipcPort ?? DEFAULT_IPC_PORT;
                patch.consoleBufferSize = settings.consoleBufferSize ?? 2000;
                patch.autoGeneratePasswords = settings.autoGeneratePasswords ?? true;
                patch.portConflictCheck = settings.portConflictCheck ?? true;
                patch.autoPortCheck = settings.autoPortCheck ?? false;
                patch.osNotifications = settings.osNotifications ?? true;
                patch.sounds = { ...DEFAULT_SOUNDS, ...settings.sounds };
                for (const [k, v] of Object.entries(APP_OPTION_DEFAULTS)) patch[k] = settings[k as AppOptionKey] ?? v;

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
                patch.consoleTokenColors = settings.consoleTokenColors ?? THEME_DEFAULTS.consoleTokenColors;
                patch.windowBackdrop = { ...WINDOW_BACKDROP_DEFAULTS, ...settings.windowBackdrop };
                patch.systemMonitorExpanded = settings.systemMonitorExpanded ?? false;
                patch.monitorItems = Array.isArray(settings.monitorItems)
                    ? settings.monitorItems.filter((i): i is string => typeof i === 'string')
                    : null;
                patch.systemMonitorMode = SYSTEM_MONITOR_MODES.includes(settings.systemMonitorMode as SystemMonitorMode)
                    ? (settings.systemMonitorMode as SystemMonitorMode)
                    : 'docked';
                patch.systemMonitorFloat = isMonitorFloat(settings.systemMonitorFloat) ? settings.systemMonitorFloat : null;
                patch.closedGuides = Array.isArray(settings.closedGuides)
                    ? settings.closedGuides.filter((i): i is string => typeof i === 'string')
                    : [];
                patch.daemonView = settings.daemonView === 'unified' ? 'unified' : 'switch';
                patch.mainDaemon =
                    typeof settings.mainDaemon === 'string' && settings.mainDaemon ? settings.mainDaemon : null;
            }
            const path = await window.api.settingsGetPath();
            patch.settingsPath = path;
            if (!isTest) console.log('[Settings] GUI settings loaded from:', path);

            patch.settingsReady = true;
            applyingLoad = true;
            try {
                set(patch);
            } finally {
                applyingLoad = false;
            }
            for (const k of Object.keys(patch)) dirtyKeys.delete(k as SettingKey);
            // 셸이 창을 만들 때 깐 효과와 화면 바탕을 맞춘다 (설정이 없으면 기본값)
            void applyWindowBackdrop(get().windowBackdrop);
            if (settings) {
                // config.json 값을 화면에 적용하고 캐시도 맞춘다
                setTheme(patch.theme as Parameters<typeof setTheme>[0]);
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
                    consoleTokenColors: patch.consoleTokenColors,
                    sidebarCompact: patch.sidebarCompact,
                });
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
            if (failed(res)) return false;
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
        const keys = [...dirtyKeys];
        if (keys.length === 0) return true;
        dirtyKeys.clear();
        try {
            const saved = await window.api.settingsSave(Object.fromEntries(keys.map((k) => [k, state[k]])));
            if (saved === false) throw new Error('settings not saved');
            console.log('[Settings] GUI settings saved');
            return true;
        } catch (error) {
            // 저장하지 못한 칸은 다음 저장에 다시 싣는다
            for (const k of keys) dirtyKeys.add(k);
            console.error('[Settings] Failed to save GUI settings:', error);
            return false;
        }
    },

    update: (partial) => {
        set(partial);
        // Debounced save is handled by subscription below
    },

    // Test-only: reset to initial state
    _resetForTest: () => {
        set({
            autoRefresh: true,
            refreshInterval: 2000,
            ipcPort: DEFAULT_IPC_PORT,
            consoleBufferSize: 2000,
            autoGeneratePasswords: true,
            portConflictCheck: true,
            autoPortCheck: false,
            osNotifications: true,
            sounds: DEFAULT_SOUNDS,
            ...APP_OPTION_DEFAULTS,
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
            windowBackdrop: WINDOW_BACKDROP_DEFAULTS,
            systemMonitorExpanded: false,
            monitorItems: null,
            systemMonitorMode: 'docked',
            systemMonitorFloat: null,
            closedGuides: [],
            daemonView: 'switch',
            mainDaemon: null,
        });
        dirtyKeys.clear();
    },
}));

// 창 불투명도는 저장값을 불러올 때도, 바꿀 때도 창에 바로 적용한다 (Windows만, 다른 곳은 API가 없다)
useSettingsStore.subscribe((state, prevState) => {
});

// ── Auto-save subscription (Zustand v5 single-listener) ──
let settingsSaveTimer: ReturnType<typeof setTimeout> | undefined;
const _settingsKeys: SettingKey[] = [
    'autoRefresh',
    'refreshInterval',
    'ipcPort',
    'consoleBufferSize',
    'autoGeneratePasswords',
    'portConflictCheck',
    'autoPortCheck',
    'osNotifications',
    'sounds',
    ...(Object.keys(APP_OPTION_DEFAULTS) as AppOptionKey[]),
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
    'consoleTokenColors',
    'windowBackdrop',
    'systemMonitorExpanded',
    'monitorItems',
    'systemMonitorMode',
    'systemMonitorFloat',
    'closedGuides',
    'daemonView',
    'mainDaemon',
];
useSettingsStore.subscribe((state, prevState) => {
    if (applyingLoad) return;
    const changed = _settingsKeys.filter((k) => state[k] !== prevState[k]);
    for (const k of changed) dirtyKeys.add(k);
    if (!state.settingsReady || !state.settingsPath) return;
    if (changed.length === 0) return;
    console.log('[Settings] Settings changed, saving...');
    clearTimeout(settingsSaveTimer);
    settingsSaveTimer = setTimeout(async () => {
        // 자동 저장 실패는 조용히 넘기지 않는다 (G-U6)
        if ((await useSettingsStore.getState().save()) === false) safeShowToast(_t('settings.save_error'), 'error');
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
            autoPortCheck: s.autoPortCheck,
            osNotifications: s.osNotifications,
            sounds: s.sounds,
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
