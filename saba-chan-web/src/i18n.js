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
    return i18n;
}

export default i18n;
