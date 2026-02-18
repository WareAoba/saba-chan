/**
 * ExtensionContext — 범용 익스텐션 시스템 컨텍스트
 *
 * 역할:
 * 1. window.api.extensionList() (Electron IPC) → 활성 익스텐션 목록
 * 2. 활성 익스텐션의 GUI 번들을 IPC를 통해 동적 로드
 * 3. 이름 규칙: window.SabaExt{Id} (예: window.SabaExtDocker)
 * 4. 슬롯 레지스트리 관리: slotId → [Component, ...]
 * 5. i18n 로드 + 기존 i18n에 병합
 */
import React, { createContext, useContext, useState, useEffect, useCallback, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../utils/helpers';
import i18n from '../i18n';

const ExtensionContext = createContext(null);

/**
 * 익스텐션 GUI 번들 동적 로드
 * IPC를 통해 JS 소스를 받아 blob URL로 <script> 태그 삽입
 * window.SabaExt{PascalId} 전역 객체를 등록함
 */
async function loadExtensionBundle(ext) {
  const globalName = `SabaExt${pascalCase(ext.id)}`;

  // 이미 로드됨
  if (window[globalName]) {
    return window[globalName];
  }

  // GUI manifest가 없는 익스텐션
  if (!ext.gui) {
    return null;
  }

  try {
    // IPC를 통해 번들 JS 소스 가져오기 (인증 포함)
    const jsSource = await window.api.extensionGuiBundle(ext.id);
    if (!jsSource) return null;

    // blob URL로 <script> 태그 삽입
    await new Promise((resolve, reject) => {
      const blob = new Blob([jsSource], { type: 'application/javascript' });
      const url = URL.createObjectURL(blob);
      const script = document.createElement('script');
      script.src = url;
      script.async = true;
      script.onload = () => {
        URL.revokeObjectURL(url);
        resolve();
      };
      script.onerror = () => {
        URL.revokeObjectURL(url);
        reject(new Error(`Failed to execute bundle for '${ext.id}'`));
      };
      document.head.appendChild(script);
    });

    // CSS 로드 (있으면)
    const cssSource = await window.api.extensionGuiStyles(ext.id);
    if (cssSource) {
      const style = document.createElement('style');
      style.textContent = cssSource;
      style.dataset.extension = ext.id;
      document.head.appendChild(style);
    }

    return window[globalName] || null;
  } catch (e) {
    console.warn(`[Extension] Failed to load GUI bundle for '${ext.id}':`, e);
    return null;
  }
}

function pascalCase(str) {
  return str.replace(/(^|[-_])(\w)/g, (_, __, c) => c.toUpperCase());
}

/**
 * 익스텐션 i18n 번역 로드 & i18next에 병합
 */
async function loadExtensionI18n(ext, lang) {
  try {
    const translations = await window.api.extensionI18n(ext.id, lang);
    if (translations && typeof translations === 'object') {
      i18n.addResourceBundle(lang, `ext_${ext.id}`, translations, true, true);
    }
  } catch (e) {
    console.warn(`[Extension] Failed to load i18n for '${ext.id}' (${lang}):`, e);
  }
}

export function ExtensionProvider({ children }) {
  const { t } = useTranslation();
  const [extensions, setExtensions] = useState([]);
  const [enabledExtensions, setEnabledExtensions] = useState([]);
  const [slots, setSlots] = useState({});
  const [loading, setLoading] = useState(true);
  const loadedRef = useRef(new Set());

  // 익스텐션 목록 가져오기
  const fetchExtensions = useCallback(async () => {
    try {
      const data = await window.api.extensionList();
      setExtensions(data.extensions || []);
    } catch (e) {
      console.warn('[Extension] Failed to fetch extensions:', e);
    }
  }, []);

  // 초기 로드
  useEffect(() => {
    fetchExtensions();
  }, [fetchExtensions]);

  // 활성 익스텐션 변경 시 GUI 번들 로드 & 슬롯 등록
  useEffect(() => {
    const enabled = extensions.filter(ext => ext.enabled);
    setEnabledExtensions(enabled);

    const loadBundles = async () => {
      const newSlots = {};

      for (const ext of enabled) {
        // 이미 로드된 경우 스킵
        if (loadedRef.current.has(ext.id)) {
          // 기존 슬롯 유지를 위해 전역 객체에서 다시 가져옴
          const globalName = `SabaExt${pascalCase(ext.id)}`;
          const mod = window[globalName];
          if (mod?.registerSlots) {
            const extSlots = mod.registerSlots();
            for (const [slotId, components] of Object.entries(extSlots)) {
              if (!newSlots[slotId]) newSlots[slotId] = [];
              newSlots[slotId].push(...(Array.isArray(components) ? components : [components]));
            }
          }
          continue;
        }

        // GUI 번들 로드
        const mod = await loadExtensionBundle(ext);
        loadedRef.current.add(ext.id);

        if (mod?.registerSlots) {
          const extSlots = mod.registerSlots();
          for (const [slotId, components] of Object.entries(extSlots)) {
            if (!newSlots[slotId]) newSlots[slotId] = [];
            newSlots[slotId].push(...(Array.isArray(components) ? components : [components]));
          }
        }

        // i18n 로드
        const currentLang = i18n.language || 'en';
        await loadExtensionI18n(ext, currentLang);
        if (currentLang !== 'en') {
          await loadExtensionI18n(ext, 'en'); // fallback
        }
      }

      setSlots(newSlots);
      setLoading(false);
    };

    loadBundles();
  }, [extensions]);

  // 익스텐션 활성/비활성 토글
  const toggleExtension = useCallback(async (extId, enable) => {
    const action = enable ? 'enable' : 'disable';
    try {
      let result;
      if (enable) {
        result = await window.api.extensionEnable(extId);
      } else {
        result = await window.api.extensionDisable(extId);
      }

      if (result && result.success === false) {
        // 구조화된 에러 응답 처리
        const code = result.error_code || 'unknown';
        const related = (result.related || []).join(', ');
        let msg;

        switch (code) {
          case 'not_found':
            msg = t('extensions.error_not_found', { id: extId, defaultValue: `Extension '${extId}' not found.` });
            break;
          case 'dependency_missing':
            msg = t('extensions.error_dep_missing', { id: extId, dep: related, defaultValue: `Cannot enable '${extId}': required extension '${related}' is not installed.` });
            break;
          case 'dependency_not_enabled':
            msg = t('extensions.error_dep_disabled', { id: extId, dep: related, defaultValue: `Cannot enable '${extId}': required extension '${related}' is not enabled. Enable it first.` });
            break;
          case 'has_dependents':
            msg = t('extensions.error_has_dependents', { id: extId, deps: related, defaultValue: `Cannot disable '${extId}': other extensions depend on it (${related}). Disable them first.` });
            break;
          case 'in_use':
            msg = t('extensions.error_in_use', { id: extId, instances: related, defaultValue: `Cannot disable '${extId}': in use by server instance(s): ${related}. Remove usage first.` });
            break;
          default:
            msg = result.error || t('extensions.error_unknown', { defaultValue: 'An unknown error occurred.' });
        }

        safeShowToast(msg, 'error', 5000);
        console.warn(`[Extension] ${action} '${extId}' failed [${code}]:`, result.error);
        return false;
      }

      await fetchExtensions(); // 목록 새로 가져오기
      return true;
    } catch (e) {
      const msg = t('extensions.error_network', { action, id: extId, defaultValue: `Failed to ${action} extension '${extId}'. Check daemon connection.` });
      safeShowToast(msg, 'error', 4000);
      console.warn(`[Extension] Failed to ${action} '${extId}':`, e);
      return false;
    }
  }, [fetchExtensions, t]);

  const value = {
    extensions,
    enabledExtensions,
    slots,
    loading,
    toggleExtension,
    refreshExtensions: fetchExtensions,
  };

  return (
    <ExtensionContext.Provider value={value}>
      {children}
    </ExtensionContext.Provider>
  );
}

export function useExtensions() {
  const ctx = useContext(ExtensionContext);
  if (!ctx) {
    // Context 없이 사용되는 경우 (단독 테스트 등) 기본값 반환
    return { extensions: [], enabledExtensions: [], slots: {}, loading: false, toggleExtension: () => {}, refreshExtensions: () => {} };
  }
  return ctx;
}

export default ExtensionContext;
