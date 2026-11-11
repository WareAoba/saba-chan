/**
 * Theme management utility
 * Supports 'auto' (follow system), 'light', and 'dark' modes
 * + accent color, gradient toggle, font scale, transition toggle,
 *   console background/text color/syntax highlight toggle
 */

import { TOKENS } from './syntaxHighlight';

// localStorage는 첫 화면 캐시일 뿐이다. 원천은 config.json(gui.theme 등, useSettingsStore).
const THEME_KEY = 'saba-chan-theme';
const CUSTOM_THEME_KEY = 'saba-chan-theme-custom';

// ── Default accent colors (original purple) ──
const DEFAULT_ACCENT = '#667eea';
const DEFAULT_ACCENT_SECONDARY = '#764ba2';

/** 테마 모드 */
export type ThemeMode = 'auto' | 'light' | 'dark';

/** 사용자 지정 테마 값 */
export interface ThemeConfig {
    accentColor: string;
    accentSecondary: string;
    useGradient: boolean;
    fontScale: number;
    enableTransitions: boolean;
    consoleSyntaxHighlight: boolean;
    consoleBgColor: string;
    consoleTextColor: string;
    consoleFontScale: number;
    consoleTokenColors: Record<string, string>;
    sidebarCompact: boolean;
}

/** Default customization values */
export const THEME_DEFAULTS: ThemeConfig = {
    accentColor: DEFAULT_ACCENT,
    accentSecondary: DEFAULT_ACCENT_SECONDARY,
    useGradient: true,
    fontScale: 100,
    enableTransitions: true,
    consoleSyntaxHighlight: true,
    consoleBgColor: '#1e1e2e',
    consoleTextColor: '#cdd6f4',
    consoleFontScale: 100,
    /** 콘솔 구문 강조 색: 토큰 → '#rrggbb'. 빈 값·없는 토큰은 테마 기본색(tokens.css --tok-*) */
    consoleTokenColors: {},
    sidebarCompact: false,
};

// ── Preset accent palettes ──
export const ACCENT_PRESETS = [
    { name: 'purple', primary: '#667eea', secondary: '#764ba2' },
    { name: 'blue',   primary: '#3b82f6', secondary: '#7c3aed' },
    { name: 'green',  primary: '#16a34a', secondary: '#0d9488' },
    { name: 'orange', primary: '#ea580c', secondary: '#dc2626' },
    { name: 'pink',   primary: '#ec4899', secondary: '#f43f5e' },
    { name: 'cyan',   primary: '#0891b2', secondary: '#0c4a6e' },
    { name: 'red',    primary: '#dc2626', secondary: '#9f1239' },
    { name: 'teal',   primary: '#0d9488', secondary: '#065f46' },
];

// ═══════════════════════════════════════════════════════════════
// 1. Light/Dark theme (existing)
// ═══════════════════════════════════════════════════════════════

/**
 * Get the saved theme preference
 * @returns {'auto' | 'light' | 'dark'}
 */
export function getTheme() {
    return (localStorage.getItem(THEME_KEY) as ThemeMode | null) || 'auto';
}

/**
 * Save and apply theme preference
 * @param {'auto' | 'light' | 'dark'} theme
 */
export function setTheme(theme: ThemeMode) {
    localStorage.setItem(THEME_KEY, theme);
    applyTheme(theme);
}

/**
 * Apply theme to the document
 * @param {'auto' | 'light' | 'dark'} theme
 */
function applyTheme(theme: ThemeMode) {
    document.documentElement.setAttribute('data-theme', theme);
    syncWindowBackground();
}

/** 원래 투명한 셸 창 (트레이 팝업·바탕화면 오버레이): 바탕을 칠하거나 반투명을 깔지 않는다 */
const transparentWindow = (root: HTMLElement) => root.dataset.trayPopup !== undefined || root.dataset.desktopOverlay !== undefined;

/**
 * 셸 창의 바탕을 테마 바탕색(--bg-0)으로 칠한다.
 * 창 크기를 바꾸는 사이 웹뷰가 아직 못 그린 자리가 흰색 대신 이 색으로 보인다.
 * 트레이 팝업·바탕화면 오버레이는 투명 창이라 칠하지 않는다.
 */
function syncWindowBackground() {
    const root = document.documentElement;
    if (transparentWindow(root)) return;
    const color = getComputedStyle(root).getPropertyValue('--bg-0').trim();
    if (color) window.api?.windowSetBackground?.(color);
}

/**
 * Get the effective (resolved) theme considering system preference
 * @returns {'light' | 'dark'}
 */
export function getEffectiveTheme() {
    const theme = getTheme();
    if (theme === 'auto') {
        return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
    }
    return theme;
}

/**
 * Initialize theme on app startup
 * Sets up the data-theme attribute and system preference listener
 */
export function initTheme() {
    const theme = getTheme();
    applyTheme(theme);
    // "시스템 따라"면 시스템 테마가 바뀔 때 창 바탕도 따라 바꾼다
    window.matchMedia?.('(prefers-color-scheme: dark)').addEventListener?.('change', syncWindowBackground);
    // 다른 창(메인 창 설정)에서 바꾸면 이 창(바탕화면 펫·트레이 팝업)도 따른다
    window.addEventListener('storage', (e) => {
        if (e.key === THEME_KEY) applyTheme(getTheme());
        else if (e.key === CUSTOM_THEME_KEY) applyCustomTheme(getCustomTheme());
    });
    // Apply saved customizations
    const custom = getCustomTheme();
    applyCustomTheme(custom);
}

// ═══════════════════════════════════════════════════════════════
// 2. Custom theme properties
// ═══════════════════════════════════════════════════════════════

/**
 * Get saved custom theme config
 * @returns {typeof THEME_DEFAULTS}
 */
export function getCustomTheme() {
    try {
        const raw = localStorage.getItem(CUSTOM_THEME_KEY);
        if (!raw) return { ...THEME_DEFAULTS };
        const parsed = JSON.parse(raw);
        return { ...THEME_DEFAULTS, ...parsed };
    } catch {
        return { ...THEME_DEFAULTS };
    }
}

/**
 * Save and apply custom theme config
 * @param {Partial<typeof THEME_DEFAULTS>} partial
 */
export function setCustomTheme(partial: Partial<ThemeConfig>) {
    const current = getCustomTheme();
    const merged = { ...current, ...partial };
    localStorage.setItem(CUSTOM_THEME_KEY, JSON.stringify(merged));
    applyCustomTheme(merged);
    return merged;
}

/**
 * Reset custom theme to defaults
 */
/** 브라우저에 남은 테마 모드 캐시를 지운다 (초기화 뒤 예전 값이 되살아나지 않게) */
export function clearStoredTheme() {
    try {
        localStorage.removeItem(THEME_KEY);
    } catch {
        /* 저장소를 못 쓰면 지울 것도 없다 */
    }
}

export function resetCustomTheme() {
    localStorage.removeItem(CUSTOM_THEME_KEY);
    applyCustomTheme(THEME_DEFAULTS);
    return { ...THEME_DEFAULTS };
}

// ── Color utilities ──────────────────────────────────────────

/**
 * Parse hex color to { r, g, b }
 */
function hexToRgb(hex: string) {
    const h = hex.replace('#', '');
    return {
        r: parseInt(h.substring(0, 2), 16),
        g: parseInt(h.substring(2, 4), 16),
        b: parseInt(h.substring(4, 6), 16),
    };
}

/**
 * Darken a hex color by a factor (0-1)
 */
function darkenHex(hex: string, factor: number) {
    const { r, g, b } = hexToRgb(hex);
    const d = 1 - factor;
    const nr = Math.round(r * d);
    const ng = Math.round(g * d);
    const nb = Math.round(b * d);
    return `#${nr.toString(16).padStart(2, '0')}${ng.toString(16).padStart(2, '0')}${nb.toString(16).padStart(2, '0')}`;
}

/**
 * Lighten a hex color by mixing with white
 */
function lightenHex(hex: string, factor: number) {
    const { r, g, b } = hexToRgb(hex);
    const nr = Math.round(r + (255 - r) * factor);
    const ng = Math.round(g + (255 - g) * factor);
    const nb = Math.round(b + (255 - b) * factor);
    return `#${nr.toString(16).padStart(2, '0')}${ng.toString(16).padStart(2, '0')}${nb.toString(16).padStart(2, '0')}`;
}

/**
 * Desaturate a hex color by a factor (0-1). Mixes toward gray of equal luminance.
 */
function desaturateHex(hex: string, factor: number) {
    const { r, g, b } = hexToRgb(hex);
    const gray = Math.round(0.299 * r + 0.587 * g + 0.114 * b);
    const nr = Math.round(r + (gray - r) * factor);
    const ng = Math.round(g + (gray - g) * factor);
    const nb = Math.round(b + (gray - b) * factor);
    return `#${nr.toString(16).padStart(2, '0')}${ng.toString(16).padStart(2, '0')}${nb.toString(16).padStart(2, '0')}`;
}

// ── WCAG 2.1 contrast utilities ─────────────────────────────

/**
 * Convert sRGB channel (0-255) to linear RGB
 */
function srgbToLinear(c: number) {
    const s = c / 255;
    return s <= 0.04045 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
}

/** #rgb·#rrggbb·rgb(r, g, b) → #rrggbb (그 밖은 null) */
function toHex6(color: unknown) {
    const c = String(color || '').trim();
    if (/^#[0-9a-f]{6}$/i.test(c)) return c;
    if (/^#[0-9a-f]{3}$/i.test(c)) return `#${[...c.slice(1)].map((x) => x + x).join('')}`;
    const m = c.match(/^rgba?\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})/i);
    if (!m) return null;
    return `#${m.slice(1, 4).map((x) => Math.min(255, Number(x)).toString(16).padStart(2, '0')).join('')}`;
}

/**
 * Compute relative luminance per WCAG 2.1
 */
function relativeLuminance(hex: string) {
    const { r, g, b } = hexToRgb(hex);
    return 0.2126 * srgbToLinear(r) + 0.7152 * srgbToLinear(g) + 0.0722 * srgbToLinear(b);
}

/**
 * Determine optimal text color (light or dark) for a given background hex.
 * Returns '#ffffff' or '#1a1a2e'.
 *
 * WCAG 2.1의 contrast ratio 공식은 채도 높은 중간 밝기 색상에서 dark text를
 * 과도하게 선호하는 알려진 한계가 있음 (Helmholtz–Kohlrausch 효과 미반영).
 * 순수 ratio 비교 대신 perceptual luminance threshold(0.36)를 사용하여
 * Material Design / Tailwind 등 주요 디자인 시스템과 동일한 결과를 제공한다.
 *
 * - lum ≤ 0.40 → white text (WCAG AA Large Text ≥3:1 충족)
 * - lum > 0.40  → dark text
 *
 * @param {string} bgHex - Background color in '#rrggbb' format
 * @returns {'#ffffff' | '#1a1a2e'}
 */
export function getContrastColor(bgHex: string) {
    const lum = relativeLuminance(bgHex);
    return lum > 0.40 ? '#1a1a2e' : '#ffffff';
}

/**
 * Apply all custom theme properties to CSS variables
 * @param {typeof THEME_DEFAULTS} config
 */
export function applyCustomTheme(config: ThemeConfig) {
    const root = document.documentElement;
    const {
        accentColor,
        accentSecondary,
        useGradient,
        fontScale,
        enableTransitions,
        consoleBgColor,
        consoleTextColor,
        consoleSyntaxHighlight,
        consoleFontScale = 100,
        consoleTokenColors = {},
        sidebarCompact = false,
    } = config;
    // 왼쪽 목록 칸을 좁고 촘촘하게 (서버·디스코드·작업 기록·설정)
    root.dataset.sidebarCompact = sidebarCompact ? 'on' : 'off';

    // ── Accent color derivations ──
    const { r, g, b } = hexToRgb(accentColor);
    const hover = darkenHex(accentColor, 0.12);

    // Brand primary / secondary
    root.style.setProperty('--brand-primary', accentColor);
    root.style.setProperty('--brand-primary-rgb', `${r}, ${g}, ${b}`);
    root.style.setProperty('--brand-secondary', accentSecondary);
    root.style.setProperty('--brand-hover', hover);

    // Text color on brand backgrounds (WCAG AA contrast)
    const brandText = getContrastColor(accentColor);
    root.style.setProperty('--brand-text', brandText);

    // Gradient vs flat
    // 순환 버튼(grad-cycle.css)은 켜졌을 때만, 두 끝 색으로 그린다
    root.dataset.brandGradient = useGradient ? 'on' : 'off';
    root.style.setProperty('--brand-from', accentColor);
    root.style.setProperty('--brand-to', accentSecondary);
    if (useGradient) {
        root.style.setProperty('--brand-gradient', `linear-gradient(in oklch 135deg, ${accentColor} 0%, ${accentSecondary} 100%)`);
    } else {
        root.style.setProperty('--brand-gradient', accentColor);
    }

    // Shadows derived from accent
    root.style.setProperty('--brand-shadow', `rgba(${r}, ${g}, ${b}, 0.4)`);
    root.style.setProperty('--brand-shadow-light', `rgba(${r}, ${g}, ${b}, 0.2)`);
    root.style.setProperty('--brand-shadow-subtle', `rgba(${r}, ${g}, ${b}, 0.1)`);
    root.style.setProperty('--brand-shadow-medium', `rgba(${r}, ${g}, ${b}, 0.3)`);
    root.style.setProperty('--brand-shadow-strong', `rgba(${r}, ${g}, ${b}, 0.45)`);

    // Light accent backgrounds (adaptive for light/dark)
    const effectiveTheme = getEffectiveTheme();
    if (effectiveTheme === 'light') {
        // Desaturate + lighten for soft pastel tints that blend with white backgrounds
        const softAccent = desaturateHex(accentColor, 0.4);
        root.style.setProperty('--brand-bg-light', lightenHex(softAccent, 0.95));
        root.style.setProperty('--brand-bg-subtle', lightenHex(softAccent, 0.92));
        root.style.setProperty('--brand-bg-muted', lightenHex(softAccent, 0.86));
        root.style.setProperty('--brand-bg-info', lightenHex(softAccent, 0.90));
        root.style.setProperty('--brand-border-light', lightenHex(softAccent, 0.86));
        root.style.setProperty('--input-focus-shadow', `rgba(${r}, ${g}, ${b}, 0.1)`);
    } else {
        const darkAccent = lightenHex(accentColor, 0.15);
        const { r: dr, g: dg, b: db } = hexToRgb(darkAccent);
        root.style.setProperty('--brand-primary', darkAccent);
        root.style.setProperty('--brand-primary-rgb', `${dr}, ${dg}, ${db}`);
        root.style.setProperty('--brand-hover', darkenHex(darkAccent, 0.08));
        // Recompute brand text for the lightened dark-mode accent
        root.style.setProperty('--brand-text', getContrastColor(darkAccent));
        root.style.setProperty('--brand-shadow', `rgba(${dr}, ${dg}, ${db}, 0.3)`);
        root.style.setProperty('--brand-shadow-light', `rgba(${dr}, ${dg}, ${db}, 0.18)`);
        root.style.setProperty('--brand-shadow-subtle', `rgba(${dr}, ${dg}, ${db}, 0.08)`);
        root.style.setProperty('--brand-shadow-medium', `rgba(${dr}, ${dg}, ${db}, 0.22)`);
        root.style.setProperty('--brand-shadow-strong', `rgba(${dr}, ${dg}, ${db}, 0.35)`);
        root.style.setProperty('--input-focus-shadow', `rgba(${dr}, ${dg}, ${db}, 0.12)`);
        if (useGradient) {
            const darkSecondary = lightenHex(accentSecondary, 0.12);
            root.style.setProperty('--brand-from', darkAccent);
            root.style.setProperty('--brand-to', darkSecondary);
            root.style.setProperty('--brand-gradient', `linear-gradient(in oklch 135deg, ${darkAccent} 0%, ${darkSecondary} 100%)`);
        } else {
            root.style.setProperty('--brand-gradient', darkAccent);
        }
    }

    // ── Font scale ──
    root.style.setProperty('--font-scale', `${fontScale / 100}`);

    // ── Transitions ──
    if (!enableTransitions) {
        root.setAttribute('data-no-transitions', 'true');
    } else {
        root.removeAttribute('data-no-transitions');
    }

    // ── Console customization ──
    root.style.setProperty('--console-custom-bg', consoleBgColor);
    root.style.setProperty('--console-custom-text', consoleTextColor);
    // 밝은 콘솔 바탕이면 조각 색 토큰을 진한 쪽으로 (tokens.css data-console-tone)
    const bgHex = toHex6(consoleBgColor);
    if (bgHex && relativeLuminance(bgHex) > 0.4) {
        root.setAttribute('data-console-tone', 'light');
    } else {
        root.removeAttribute('data-console-tone');
    }
    root.style.setProperty('--console-syntax-highlight', consoleSyntaxHighlight ? '1' : '0');
    root.style.setProperty('--console-font-scale', `${consoleFontScale / 100}`);
    // 사용자가 고른 구문 강조 색은 테마·밝은 콘솔 기본색보다 앞선다. 빈 값이면 기본색으로 돌아간다
    for (const token of TOKENS) {
        const color = consoleTokenColors?.[token];
        if (color) root.style.setProperty(`--tok-${token}`, color);
        else root.style.removeProperty(`--tok-${token}`);
    }
}

// ═══════════════════════════════════════════════════════════════
// 3. 창 바탕 반투명 (설정 → 외관, config.json gui.windowBackdrop)
// ═══════════════════════════════════════════════════════════════

/** 데몬 GuiSettings.windowBackdrop (updater/src/window_backdrop.rs)과 같은 모양·기본값 */
export interface WindowBackdropConfig {
    enabled: boolean;
    /** Windows 재질: tabbed | mica | acrylic */
    windows: string;
    /** macOS 리퀴드 글래스: regular | clear */
    mac: string;
    /** 덮는 테마 바탕의 진하기 0–100 */
    opacity: number;
    /** 덮는 바탕을 흰색(+)·검은색(-) 쪽으로 섞는 정도 -50–50 */
    brightness: number;
}

export const WINDOW_BACKDROP_DEFAULTS: Readonly<WindowBackdropConfig> = Object.freeze({
    enabled: true,
    windows: 'tabbed',
    mac: 'regular',
    opacity: 60,
    brightness: 0,
});

/** 셸이 알려 주는, 이 컴퓨터에서 쓸 수 있는 것 (window_backdrop.rs Support + 이 창에 켜졌는지) */
export interface BackdropSupport {
    platform: 'windows' | 'macos' | 'linux';
    available: boolean;
    /** windows-old | no-compositor */
    reason?: string;
    choices: string[];
    /** 창을 끌 때 느린 재질 */
    slow: string[];
    /** system | compositor | none */
    blur: string;
    /** false면 켜고 끄기가 창을 다시 열 때 적용된다 (Linux) */
    liveToggle: boolean;
    minOpacity: number;
    active: boolean;
    transparent: boolean;
}

/** 덮는 바탕 CSS 값: 진하기(흐림이 없으면 최소값까지)와 밝기(흰색·검은색 쪽으로 섞기) */
export function backdropVars(config: WindowBackdropConfig, minOpacity = 0): Record<string, string> {
    return {
        '--backdrop-opacity': `${Math.min(100, Math.max(config.opacity, minOpacity))}%`,
        '--backdrop-mix': config.brightness >= 0 ? 'white' : 'black',
        '--backdrop-mix-amount': `${Math.abs(config.brightness)}%`,
    };
}

let backdropSupport: BackdropSupport | null = null;
const backdropListeners = new Set<() => void>();

/** 마지막으로 셸이 알려 준 것 (설정 화면이 고를 거리·안내를 정한다). 셸이 없으면 null */
export function getBackdropSupport() {
    return backdropSupport;
}

export function subscribeBackdropSupport(listener: () => void) {
    backdropListeners.add(listener);
    return () => {
        backdropListeners.delete(listener);
    };
}

/**
 * 이 창에 반투명을 적용한다. 셸이 효과를 깔았다고 답했을 때만 바탕을 비운다 (data-backdrop="on", styles/backdrop.css).
 * 트레이 팝업·바탕화면 오버레이는 원래 투명 창이라 건드리지 않는다
 */
export async function applyWindowBackdrop(config: WindowBackdropConfig) {
    const root = document.documentElement;
    if (transparentWindow(root)) return;
    let info: BackdropSupport | null = null;
    try {
        info = (await window.api?.windowSetBackdrop?.(config)) ?? null;
    } catch (e) {
        console.warn('[Theme] Failed to apply window backdrop:', e);
    }
    backdropSupport = info;
    for (const [k, v] of Object.entries(backdropVars(config, info?.minOpacity ?? 0))) root.style.setProperty(k, v);
    root.dataset.backdrop = info?.active ? 'on' : 'off';
    for (const l of backdropListeners) l();
}
