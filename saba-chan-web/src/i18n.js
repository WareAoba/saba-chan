import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';

export const SUPPORTED_LANGUAGES = ['ko', 'en', 'ja', 'zh-CN', 'zh-TW', 'es', 'pt-BR', 'ru', 'de', 'fr'];

// web: 이 화면의 글, common: GUI 브리지(createApi)가 만드는 오류 문구
const files = import.meta.glob(['@locales/*/web.json', '@locales/*/common.json'], { import: 'default' });

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

export async function initI18n(language = pickLanguage()) {
    const resources = {};
    for (const lang of new Set([language, 'en'])) {
        resources[lang] = {};
        for (const ns of ['web', 'common']) {
            const load = Object.entries(files).find(([k]) => k.endsWith(`/${lang}/${ns}.json`))?.[1];
            resources[lang][ns] = load ? await load() : {};
        }
    }
    await i18n.use(initReactI18next).init({
        lng: language,
        fallbackLng: 'en',
        resources,
        ns: ['web', 'common'],
        defaultNS: 'web',
        interpolation: { escapeValue: false },
    });
    document.documentElement.lang = language;
    document.title = i18n.t('app_name');
    return i18n;
}

export default i18n;

const moduleLocalesLoaded = new Set();

/** 모듈 locales(`mod_<이름>`)를 붙인다: 게임 이름·설정 이름·선택지·약관 (GUI useServerStore와 같다). 모듈 판마다 한 번 */
export async function registerModuleLocales(api, modules) {
    for (const m of modules || []) {
        const key = `${m.name}@${m.version || ''}`;
        if (moduleLocalesLoaded.has(key)) continue;
        moduleLocalesLoaded.add(key);
        const all = await api.moduleGetLocales?.(m.name);
        for (const [lang, data] of Object.entries(all || {})) {
            if (data && typeof data === 'object') i18n.addResourceBundle(lang, `mod_${m.name}`, data, true, true);
        }
    }
}
