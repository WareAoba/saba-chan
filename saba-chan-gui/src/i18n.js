import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
// 영어는 폴백이라 번들에 넣고, 나머지 언어는 그 언어로 바꿀 때 불러온다 (첫 로딩 파싱을 줄인다)
import enBot from '../../locales/en/bot.json';
import enCommon from '../../locales/en/common.json';
import enGui from '../../locales/en/gui.json';

// Single Source of Truth — Rust constants 모듈과 동일한 목록
const SUPPORTED_LANGUAGES = ['en', 'ko', 'ja', 'zh-CN', 'zh-TW', 'es', 'pt-BR', 'ru', 'de', 'fr'];

const localeFiles = import.meta.glob(
    ['../../locales/*/common.json', '../../locales/*/gui.json', '../../locales/*/bot.json', '!../../locales/en/*'],
    { import: 'default' },
);

/** i18next 백엔드: 언어·네임스페이스 하나를 따로 묶인 JSON에서 읽는다 */
const localeBackend = {
    type: 'backend',
    init() {},
    read(language, namespace, callback) {
        const load = localeFiles[`../../locales/${language}/${namespace}.json`];
        if (!load) {
            callback(null, {});
            return;
        }
        load().then(
            (data) => callback(null, data),
            (err) => callback(err, false),
        );
    },
};

// 저장된 언어 또는 시스템 언어 가져오기
const getInitialLanguage = async () => {
    try {
        // 1. config.json의 gui.language가 단일 진실 원천 (데몬과 공유)
        if (window.electron && window.electron.getLanguage) {
            const savedLanguage = await window.electron.getLanguage();
            if (savedLanguage && SUPPORTED_LANGUAGES.includes(savedLanguage)) {
                console.log('Using language from Electron settings:', savedLanguage);
                // localStorage도 동기화 (캐시 역할)
                localStorage.setItem('i18nextLng', savedLanguage);
                return savedLanguage;
            }
        }

        // 2. Electron API 사용 불가 시 localStorage 폴백 (웹 전용 모드 등)
        const storedLang = localStorage.getItem('i18nextLng');
        if (storedLang && SUPPORTED_LANGUAGES.includes(storedLang)) {
            console.log('Using language from localStorage:', storedLang);
            return storedLang;
        }
    } catch (error) {
        console.warn('Failed to get saved language:', error);
    }

    // 3. 시스템 언어 또는 기본값
    const browserLang = navigator.language;

    // 정확한 매칭 시도
    if (SUPPORTED_LANGUAGES.includes(browserLang)) {
        console.log('Using browser language:', browserLang);
        return browserLang;
    }

    // 언어 코드만으로 매칭 시도
    const baseLang = browserLang.split('-')[0];
    const matched = SUPPORTED_LANGUAGES.find((lang) => lang.startsWith(baseLang));
    if (matched) {
        console.log('Using matched language:', matched);
        return matched;
    }

    const defaultLang = 'en';
    console.log('Using default language:', defaultLang);
    return defaultLang;
};

// 캐시된 언어를 동기적으로 읽어 초기 렌더 시 올바른 언어로 표시
const getCachedLanguage = () => {
    const cached = localStorage.getItem('i18nextLng');
    if (cached && SUPPORTED_LANGUAGES.includes(cached)) {
        return cached;
    }
    return 'en';
};

// react-i18next 플러그인을 동기적으로 등록하고 초기화해야
// React 렌더링 시점에 useTranslation 훅이 정상 동작함.
// 캐시된 언어 파일을 다 읽으면 끝나는 약속을 내보내, 첫 렌더를 그 뒤로 미룬다(영어 깜빡임 방지)
export const i18nReady = i18n
    .use(localeBackend)
    .use(initReactI18next)
    .init({
        resources: { en: { common: enCommon, gui: enGui, bot: enBot } },
        partialBundledLanguages: true,
        ns: ['gui', 'common', 'bot'],
        defaultNS: 'gui',
        lng: getCachedLanguage(), // localStorage 캐시 → 첫 실행 이후 깜빡임 없음
        fallbackLng: 'en',
        interpolation: {
            escapeValue: false, // React already escapes values
        },
        react: { useSuspense: false },
    })
    .catch((err) => {
        console.warn('[i18n] Failed to load language files:', err?.message || err);
    });

// config.json(gui)에서 실제 언어를 비동기로 가져와 동기화
getInitialLanguage().then((lang) => {
    if (lang && lang !== i18n.language) {
        i18n.changeLanguage(lang);
    }
}).catch((err) => {
    console.warn('[i18n] Language initialization failed:', err.message);
});

export default i18n;
