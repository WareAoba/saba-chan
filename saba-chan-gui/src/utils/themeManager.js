/**
 * Theme management utility
 * Supports 'auto' (follow system), 'light', and 'dark' modes
 * + accent color, gradient toggle, font scale, transition toggle,
 *   console background/text color/syntax highlight toggle
 */

const THEME_KEY = 'saba-chan-theme';
const CUSTOM_THEME_KEY = 'saba-chan-theme-custom';

// ── Default accent colors (original purple) ──
const DEFAULT_ACCENT = '#667eea';
const DEFAULT_ACCENT_SECONDARY = '#764ba2';

/** Default customization values */
export const THEME_DEFAULTS = {
    accentColor: DEFAULT_ACCENT,
    accentSecondary: DEFAULT_ACCENT_SECONDARY,
    useGradient: true,
    fontScale: 100,
    enableTransitions: true,
    consoleSyntaxHighlight: true,
    consoleBgColor: '#1e1e2e',
    consoleTextColor: '#cdd6f4',
    consoleFontScale: 100,
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
    return localStorage.getItem(THEME_KEY) || 'auto';
}

/**
 * Save and apply theme preference
 * @param {'auto' | 'light' | 'dark'} theme
 */
export function setTheme(theme) {
    localStorage.setItem(THEME_KEY, theme);
    applyTheme(theme);
}

/**
 * Apply theme to the document
 * @param {'auto' | 'light' | 'dark'} theme
 */
function applyTheme(theme) {
    document.documentElement.setAttribute('data-theme', theme);
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
export function setCustomTheme(partial) {
    const current = getCustomTheme();
    const merged = { ...current, ...partial };
    localStorage.setItem(CUSTOM_THEME_KEY, JSON.stringify(merged));
    applyCustomTheme(merged);
    return merged;
}

/**
 * Reset custom theme to defaults
 */
export function resetCustomTheme() {
    localStorage.removeItem(CUSTOM_THEME_KEY);
    applyCustomTheme(THEME_DEFAULTS);
    return { ...THEME_DEFAULTS };
}

// ── Color utilities ──────────────────────────────────────────

/**
 * Parse hex color to { r, g, b }
 */
function hexToRgb(hex) {
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
function darkenHex(hex, factor) {
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
function lightenHex(hex, factor) {
    const { r, g, b } = hexToRgb(hex);
    const nr = Math.round(r + (255 - r) * factor);
    const ng = Math.round(g + (255 - g) * factor);
    const nb = Math.round(b + (255 - b) * factor);
    return `#${nr.toString(16).padStart(2, '0')}${ng.toString(16).padStart(2, '0')}${nb.toString(16).padStart(2, '0')}`;
}

/**
 * Desaturate a hex color by a factor (0-1). Mixes toward gray of equal luminance.
 */
function desaturateHex(hex, factor) {
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
function srgbToLinear(c) {
    const s = c / 255;
    return s <= 0.04045 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
}

/**
 * Compute relative luminance per WCAG 2.1
 */
function relativeLuminance(hex) {
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
export function getContrastColor(bgHex) {
    const lum = relativeLuminance(bgHex);
    return lum > 0.40 ? '#1a1a2e' : '#ffffff';
}

/**
 * Apply all custom theme properties to CSS variables
 * @param {typeof THEME_DEFAULTS} config
 */
export function applyCustomTheme(config) {
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
    } = config;

    // ── Accent color derivations ──
    const { r, g, b } = hexToRgb(accentColor);
    const hover = darkenHex(accentColor, 0.12);

    // Brand primary / secondary
    root.style.setProperty('--brand-primary', accentColor);
    root.style.setProperty('--brand-secondary', accentSecondary);
    root.style.setProperty('--brand-hover', hover);

    // Text color on brand backgrounds (WCAG AA contrast)
    const brandText = getContrastColor(accentColor);
    root.style.setProperty('--brand-text', brandText);

    // Gradient vs flat
    if (useGradient) {
        root.style.setProperty('--brand-gradient', `linear-gradient(135deg, ${accentColor} 0%, ${accentSecondary} 100%)`);
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
        // Light mode app background: heavily desaturated + light pastel gradient
        const bgPrimary = lightenHex(desaturateHex(accentColor, 0.45), 0.78);
        const bgSecondary = lightenHex(desaturateHex(accentSecondary, 0.45), 0.78);
        root.style.setProperty('--bg-app', useGradient
            ? `linear-gradient(135deg, ${bgPrimary} 0%, ${bgSecondary} 100%)`
            : bgPrimary);
    } else {
        const darkAccent = lightenHex(accentColor, 0.15);
        const { r: dr, g: dg, b: db } = hexToRgb(darkAccent);
        root.style.setProperty('--brand-primary', darkAccent);
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
            root.style.setProperty('--brand-gradient', `linear-gradient(135deg, ${darkAccent} 0%, ${darkSecondary} 100%)`);
        } else {
            root.style.setProperty('--brand-gradient', darkAccent);
        }
        // Dark mode app background: dark gray with subtle accent tint
        const bgDarkPrimary = darkenHex(desaturateHex(accentColor, 0.45), 0.78);
        const bgDarkSecondary = darkenHex(desaturateHex(accentSecondary, 0.45), 0.78);
        root.style.setProperty('--bg-app', useGradient
            ? `linear-gradient(135deg, ${bgDarkPrimary} 0%, ${bgDarkSecondary} 100%)`
            : bgDarkPrimary);
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
    root.style.setProperty('--console-syntax-highlight', consoleSyntaxHighlight ? '1' : '0');
    root.style.setProperty('--console-font-scale', `${consoleFontScale / 100}`);
}
