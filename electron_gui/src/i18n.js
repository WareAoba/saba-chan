import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import LanguageDetector from 'i18next-browser-languagedetector';

// Import translation files
import enCommon from '../../locales/en/common.json';
import koCommon from '../../locales/ko/common.json';
import jaCommon from '../../locales/ja/common.json';
import enGui from '../../locales/en/gui.json';
import koGui from '../../locales/ko/gui.json';
import jaGui from '../../locales/ja/gui.json';

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
};

// 저장된 언어 또는 시스템 언어 가져오기
const getInitialLanguage = async () => {
  try {
    // 1. 먼저 localStorage 확인
    const storedLang = localStorage.getItem('i18nextLng');
    if (storedLang && ['en', 'ko', 'ja'].includes(storedLang)) {
      console.log('Using language from localStorage:', storedLang);
      return storedLang;
    }
    
    // 2. Electron API에서 설정된 언어 가져오기
    if (window.electron && window.electron.getLanguage) {
      const savedLanguage = await window.electron.getLanguage();
      if (savedLanguage && ['en', 'ko', 'ja'].includes(savedLanguage)) {
        console.log('Using language from Electron settings:', savedLanguage);
        return savedLanguage;
      }
    }
  } catch (error) {
    console.warn('Failed to get saved language:', error);
  }
  
  // 3. 시스템 언어 또는 기본값
  const browserLang = navigator.language.split('-')[0];
  const defaultLang = ['en', 'ko', 'ja'].includes(browserLang) ? browserLang : 'en';
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
