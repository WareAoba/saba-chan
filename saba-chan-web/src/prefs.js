/** 이 화면 설정 (언어·테마): 이 브라우저에만 둔다. 저장소를 못 쓰는 브라우저에서는 기본값으로 그린다 */
const KEY = { theme: 'saba-web.theme', language: 'saba-web.language' };
export const THEMES = ['system', 'light', 'dark'];

export function readPref(name) {
    try {
        return globalThis.localStorage?.getItem(KEY[name]) || null;
    } catch {
        return null;
    }
}

export function writePref(name, value) {
    try {
        globalThis.localStorage?.setItem(KEY[name], value);
    } catch {}
}

/** 고른 테마 → 그릴 테마. 기기 따라(`system`)나 모르는 값은 기기 설정을 따른다 */
export function resolveTheme(pref, systemLight) {
    if (pref === 'light' || pref === 'dark') return pref;
    return systemLight ? 'light' : 'dark';
}

const systemQuery = () => globalThis.matchMedia?.('(prefers-color-scheme: light)');
let unwatch = null;

/** 테마를 적용한다. 기기 따라면 기기 설정이 바뀔 때도 따라간다 */
export function applyTheme(pref = readPref('theme') || 'system') {
    const query = systemQuery();
    const paint = () => {
        document.documentElement.dataset.theme = resolveTheme(pref, !!query?.matches);
    };
    paint();
    unwatch?.();
    unwatch = null;
    if (pref !== 'light' && pref !== 'dark' && query?.addEventListener) {
        query.addEventListener('change', paint);
        unwatch = () => query.removeEventListener('change', paint);
    }
}
