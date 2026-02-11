/**
 * Theme management utility
 * Supports 'auto' (follow system), 'light', and 'dark' modes
 */

const THEME_KEY = 'saba-chan-theme';

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
export function applyTheme(theme) {
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
}
