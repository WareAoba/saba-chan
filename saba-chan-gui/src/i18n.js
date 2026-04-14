import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import deBot from '../../locales/de/bot.json';
import deCommon from '../../locales/de/common.json';
import deGui from '../../locales/de/gui.json';
// Import translation files
import enBot from '../../locales/en/bot.json';
import enCommon from '../../locales/en/common.json';
import enGui from '../../locales/en/gui.json';
import esBot from '../../locales/es/bot.json';
import esCommon from '../../locales/es/common.json';
import esGui from '../../locales/es/gui.json';
import frBot from '../../locales/fr/bot.json';
import frCommon from '../../locales/fr/common.json';
import frGui from '../../locales/fr/gui.json';
import jaBot from '../../locales/ja/bot.json';
import jaCommon from '../../locales/ja/common.json';
import jaGui from '../../locales/ja/gui.json';
import koBot from '../../locales/ko/bot.json';
import koCommon from '../../locales/ko/common.json';
import koGui from '../../locales/ko/gui.json';
import ptBrBot from '../../locales/pt-BR/bot.json';
import ptBrCommon from '../../locales/pt-BR/common.json';
import ptBrGui from '../../locales/pt-BR/gui.json';
import ruBot from '../../locales/ru/bot.json';
import ruCommon from '../../locales/ru/common.json';
import ruGui from '../../locales/ru/gui.json';
import zhCnBot from '../../locales/zh-CN/bot.json';
import zhCnCommon from '../../locales/zh-CN/common.json';
import zhCnGui from '../../locales/zh-CN/gui.json';
import zhTwBot from '../../locales/zh-TW/bot.json';
import zhTwCommon from '../../locales/zh-TW/common.json';
import zhTwGui from '../../locales/zh-TW/gui.json';

// Single Source of Truth — Rust constants 모듈과 동일한 목록
const SUPPORTED_LANGUAGES = ['en', 'ko', 'ja', 'zh-CN', 'zh-TW', 'es', 'pt-BR', 'ru', 'de', 'fr'];

const resources = {
    en: {
        common: enCommon,
        gui: enGui,
        bot: enBot,
    },
    ko: {
        common: koCommon,
        gui: koGui,
        bot: koBot,
    },
    ja: {
        common: jaCommon,
        gui: jaGui,
        bot: jaBot,
    },
    'zh-CN': {
        common: zhCnCommon,
        gui: zhCnGui,
        bot: zhCnBot,
    },
    'zh-TW': {
        common: zhTwCommon,
        gui: zhTwGui,
        bot: zhTwBot,
    },
    es: {
        common: esCommon,
        gui: esGui,
        bot: esBot,
    },
    'pt-BR': {
        common: ptBrCommon,
        gui: ptBrGui,
        bot: ptBrBot,
    },
    ru: {
        common: ruCommon,
        gui: ruGui,
        bot: ruBot,
    },
    de: {
        common: deCommon,
        gui: deGui,
        bot: deBot,
    },
    fr: {
        common: frCommon,
        gui: frGui,
        bot: frBot,
    },
};

// 저장된 언어 또는 시스템 언어 가져오기
const getInitialLanguage = async () => {
    try {
        // 1. Electron settings.json이 단일 진실 원천 (봇·데몬과 공유)
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
// React 렌더링 시점에 useTranslation 훅이 정상 동작함
i18n.use(initReactI18next).init({
    resources,
    defaultNS: 'gui',
    lng: getCachedLanguage(), // localStorage 캐시 → 첫 실행 이후 깜빡임 없음
    fallbackLng: 'en',
    interpolation: {
        escapeValue: false, // React already escapes values
    },
});

// Electron settings.json에서 실제 언어를 비동기로 가져와 동기화
getInitialLanguage().then((lang) => {
    if (lang && lang !== i18n.language) {
        i18n.changeLanguage(lang);
    }
}).catch((err) => {
    console.warn('[i18n] Language initialization failed:', err.message);
});

export default i18n;
