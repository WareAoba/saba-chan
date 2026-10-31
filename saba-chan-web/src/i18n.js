import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import { readPref, writePref } from './prefs';

export const SUPPORTED_LANGUAGES = ['ko', 'en', 'ja', 'zh-CN', 'zh-TW', 'es', 'pt-BR', 'ru', 'de', 'fr'];

// web: 이 화면의 글, common: GUI 브리지(createApi)가 만드는 오류 문구,
// gui: 앱과 같은 화면(예약·사바쨩 설정·점검 등)은 앱 문구를 그대로 쓴다 (코드에서는 `tg('키')`)
const files = import.meta.glob(['@locales/*/web.json', '@locales/*/common.json', '@locales/*/gui.json'], {
    import: 'default',
});

/** 브라우저 언어 → 지원 언어 (ko-KR → ko, pt → pt-BR, zh-HK → zh-TW) */
export function pickLanguage(tags = globalThis.navigator?.languages ?? []) {
    for (const raw of tags) {
        const tag = String(raw).toLowerCase();
        const exact = SUPPORTED_LANGUAGES.find((l) => l.toLowerCase() === tag);
        if (exact) return exact;
        const base = tag.split('-')[0];
        if (base === 'zh') return /tw|hk|mo|hant/.test(tag) ? 'zh-TW' : 'zh-CN';
        if (base === 'pt') return 'pt-BR';
        const hit = SUPPORTED_LANGUAGES.find((l) => l === base);
        if (hit) return hit;
    }
    return 'en';
}

async function bundles(lang) {
    const out = {};
    for (const ns of ['web', 'common', 'gui']) {
        const load = Object.entries(files).find(([k]) => k.endsWith(`/${lang}/${ns}.json`))?.[1];
        out[ns] = load ? await load() : {};
    }
    return out;
}

/** 이 브라우저에서 고른 언어, 없으면 브라우저 언어 */
export const startLanguage = () => {
    const saved = readPref('language');
    return SUPPORTED_LANGUAGES.includes(saved) ? saved : pickLanguage();
};

export async function initI18n(language = startLanguage()) {
    const resources = {};
    for (const lang of new Set([language, 'en'])) resources[lang] = await bundles(lang);
    await i18n.use(initReactI18next).init({
        lng: language,
        fallbackLng: 'en',
        resources,
        ns: ['web', 'common', 'gui'],
        defaultNS: 'web',
        interpolation: { escapeValue: false },
        // 모듈 locales를 나중에 붙여도 번역된 이름으로 다시 그린다 (registerModuleLocales)
        react: { bindI18nStore: 'added' },
    });
    document.documentElement.lang = language;
    document.title = i18n.t('app_name');
    return i18n;
}

/** 이 화면의 언어를 바꾸고 이 브라우저에 기억한다 */
export async function setLanguage(language) {
    if (!SUPPORTED_LANGUAGES.includes(language)) return;
    if (!i18n.hasResourceBundle(language, 'web')) {
        for (const [ns, data] of Object.entries(await bundles(language))) {
            i18n.addResourceBundle(language, ns, data, true, true);
        }
    }
    await i18n.changeLanguage(language);
    writePref('language', language);
    document.documentElement.lang = language;
    document.title = i18n.t('app_name');
}

export default i18n;

const moduleLocalesLoaded = new Set();

/** 모듈 locales(`mod_<이름>`)를 붙인다: 게임 이름·설정 이름·선택지·약관 (GUI useServerStore와 같다).
 * 모듈 판마다 한 번이고, 받지 못한 모듈은 다음 목록 때 다시 받는다. 붙이면 화면이 다시 그려진다(`bindI18nStore`) */
export async function registerModuleLocales(api, modules) {
    const pending = (modules || []).filter((m) => !moduleLocalesLoaded.has(`${m.name}@${m.version || ''}`));
    await Promise.all(
        pending.map(async (m) => {
            const all = await api.moduleGetLocales?.(m.name);
            const entries = Object.entries(all || {}).filter(([, data]) => data && typeof data === 'object');
            if (entries.length === 0) return;
            for (const [lang, data] of entries) i18n.addResourceBundle(lang, `mod_${m.name}`, data, true, true);
            moduleLocalesLoaded.add(`${m.name}@${m.version || ''}`);
        }),
    );
}
