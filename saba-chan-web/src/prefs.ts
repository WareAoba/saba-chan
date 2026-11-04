import { applyConsoleFonts, normalizeConsoleFonts } from '@gui/utils/consoleFonts';

/** 이 화면 설정 (언어·테마·콘솔 글꼴): 이 브라우저에만 둔다. 저장소를 못 쓰는 브라우저에서는 기본값으로 그린다 */
const KEY = { theme: 'saba-web.theme', language: 'saba-web.language', consoleFonts: 'saba-web.consoleFonts' };
type PrefName = keyof typeof KEY;
export const THEMES = ['system', 'light', 'dark'];

export function readPref(name: PrefName) {
    try {
        return globalThis.localStorage?.getItem(KEY[name]) || null;
    } catch {
        return null;
    }
}

export function writePref(name: PrefName, value: string) {
    try {
        globalThis.localStorage?.setItem(KEY[name], value);
    } catch {}
}

/** 고른 테마 → 그릴 테마. 기기 따라(`system`)나 모르는 값은 기기 설정을 따른다 */
export function resolveTheme(pref: string | null, systemLight: boolean) {
    if (pref === 'light' || pref === 'dark') return pref;
    return systemLight ? 'light' : 'dark';
}

const systemQuery = () => globalThis.matchMedia?.('(prefers-color-scheme: light)');
let unwatch: (() => void) | null = null;

/** 테마를 적용한다. 기기 따라면 기기 설정이 바뀔 때도 따라간다 */
export function applyTheme(pref: string = readPref('theme') || 'system') {
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

/** 콘솔 글꼴 (앱과 같은 후보·기본값, @gui/utils/consoleFonts). 깨졌거나 없는 칸은 기본값 */
export function readConsoleFonts() {
    try {
        return normalizeConsoleFonts(JSON.parse(readPref('consoleFonts') || '{}'));
    } catch {
        return normalizeConsoleFonts({});
    }
}

/** 콘솔 글꼴을 기억하고 그린다. 한자는 화면 언어에 따라 (중국어면 중국어 글꼴 먼저) */
export function applyScreenFonts(fonts: ReturnType<typeof normalizeConsoleFonts> = readConsoleFonts(), language: string = document.documentElement.lang) {
    writePref('consoleFonts', JSON.stringify(fonts));
    applyConsoleFonts(fonts, language);
}
