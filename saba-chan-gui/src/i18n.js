import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import LanguageDetector from 'i18next-browser-languagedetector';

// Import translation files
import enCommon from '../../locales/en/common.json';
import koCommon from '../../locales/ko/common.json';
import jaCommon from '../../locales/ja/common.json';
import zhCnCommon from '../../locales/zh-CN/common.json';
import zhTwCommon from '../../locales/zh-TW/common.json';
import esCommon from '../../locales/es/common.json';
import ptBrCommon from '../../locales/pt-BR/common.json';
import ruCommon from '../../locales/ru/common.json';
import deCommon from '../../locales/de/common.json';
import frCommon from '../../locales/fr/common.json';
import enGui from '../../locales/en/gui.json';
import koGui from '../../locales/ko/gui.json';
import jaGui from '../../locales/ja/gui.json';
import zhCnGui from '../../locales/zh-CN/gui.json';
import zhTwGui from '../../locales/zh-TW/gui.json';
import esGui from '../../locales/es/gui.json';
import ptBrGui from '../../locales/pt-BR/gui.json';
import ruGui from '../../locales/ru/gui.json';
import deGui from '../../locales/de/gui.json';
import frGui from '../../locales/fr/gui.json';

const resources = {
  en: {
    common: enCommon,
    gui: enGui,
  },
  ko: {
    common: koCommon,
    gui: koGui,
  },
  ja: {
    common: jaCommon,
    gui: jaGui,
  },
  'zh-CN': {
    common: zhCnCommon,
    gui: zhCnGui,
  },
  'zh-TW': {
    common: zhTwCommon,
    gui: zhTwGui,
  },
  es: {
    common: esCommon,
    gui: esGui,
  },
  'pt-BR': {
    common: ptBrCommon,
    gui: ptBrGui,
  },
  ru: {
    common: ruCommon,
    gui: ruGui,
  },
  de: {
    common: deCommon,
    gui: deGui,
  },
  fr: {
    common: frCommon,
    gui: frGui,
  },
};

// 저장된 언어 또는 시스템 언어 가져오기
const getInitialLanguage = async () => {
  try {
    // 1. 먼저 localStorage 확인
    const storedLang = localStorage.getItem('i18nextLng');
    const supportedLanguages = ['en', 'ko', 'ja', 'zh-CN', 'zh-TW', 'es', 'pt-BR', 'ru', 'de', 'fr'];
    if (storedLang && supportedLanguages.includes(storedLang)) {
      console.log('Using language from localStorage:', storedLang);
      return storedLang;
    }
    
    // 2. Electron API에서 설정된 언어 가져오기
    if (window.electron && window.electron.getLanguage) {
      const savedLanguage = await window.electron.getLanguage();
      if (savedLanguage && supportedLanguages.includes(savedLanguage)) {
        console.log('Using language from Electron settings:', savedLanguage);
        return savedLanguage;
      }
    }
  } catch (error) {
    console.warn('Failed to get saved language:', error);
  }
  
  // 3. 시스템 언어 또는 기본값
  const browserLang = navigator.language;
  const supportedLanguages = ['en', 'ko', 'ja', 'zh-CN', 'zh-TW', 'es', 'pt-BR', 'ru', 'de', 'fr'];
  
  // 정확한 매칭 시도
  if (supportedLanguages.includes(browserLang)) {
    console.log('Using browser language:', browserLang);
    return browserLang;
  }
  
  // 언어 코드만으로 매칭 시도
  const baseLang = browserLang.split('-')[0];
  const matched = supportedLanguages.find(lang => lang.startsWith(baseLang));
  if (matched) {
    console.log('Using matched language:', matched);
    return matched;
  }
  
  const defaultLang = 'en';
  console.log('Using default language:', defaultLang);
  return defaultLang;
};

// i18n 초기화 (비동기)
const initI18n = async () => {
  const initialLanguage = await getInitialLanguage();
  
  await i18n
    .use(initReactI18next) // Pass i18n instance to react-i18next (LanguageDetector 제거)
    .init({
      resources,
      defaultNS: 'gui',
      lng: initialLanguage, // 저장된 언어로 초기화
      fallbackLng: 'en',
      
      interpolation: {
        escapeValue: false, // React already escapes values
      },
    });
};

// 초기화 실행
initI18n();

export default i18n;
