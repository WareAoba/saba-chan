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
import { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import builtinExtensions from '../builtinExtensions';
import i18n from '../i18n';
import { safeShowToast } from '../utils/helpers';

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

    // ── 레지스트리 / 버전관리 상태 ──────────────────────────────
    /** 원격 레지스트리에서 받아온 가용 익스텐션 목록 */
    const [registryExtensions, setRegistryExtensions] = useState([]);
    /** 업데이트 가능한 익스텐션 목록 */
    const [availableUpdates, setAvailableUpdates] = useState([]);
    /** 레지스트리 페치 중 여부 */
    const [registryLoading, setRegistryLoading] = useState(false);
    /** 현재 설치 진행 중인 익스텐션 ID 집합 */
    const [installingIds, setInstallingIds] = useState(new Set());

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
        const enabled = extensions.filter((ext) => ext.enabled);
        setEnabledExtensions(enabled);

        const loadBundles = async () => {
            const newSlots = {};

            /** 슬롯 맵을 newSlots에 병합하는 헬퍼 */
            const mergeSlots = (extSlots) => {
                for (const [slotId, components] of Object.entries(extSlots)) {
                    if (!newSlots[slotId]) newSlots[slotId] = [];
                    newSlots[slotId].push(...(Array.isArray(components) ? components : [components]));
                }
            };

            for (const ext of enabled) {
                // ① 내장 익스텐션: 정적 import에서 바로 슬롯 등록 (UMD 불필요)
                const builtin = builtinExtensions[ext.id];
                if (builtin) {
                    if (builtin.registerSlots) {
                        mergeSlots(builtin.registerSlots());
                    }
                    // 내장이라도 i18n은 데몬이 제공할 수 있으므로 로드 시도
                    if (!loadedRef.current.has(ext.id)) {
                        loadedRef.current.add(ext.id);
                        const currentLang = i18n.language || 'en';
                        await loadExtensionI18n(ext, currentLang);
                        if (currentLang !== 'en') {
                            await loadExtensionI18n(ext, 'en');
                        }
                    }
                    continue;
                }

                // ② 이미 로드된 외부 익스텐션
                if (loadedRef.current.has(ext.id)) {
                    const globalName = `SabaExt${pascalCase(ext.id)}`;
                    const mod = window[globalName];
                    if (mod?.registerSlots) {
                        mergeSlots(mod.registerSlots());
                    }
                    continue;
                }

                // ③ 외부 익스텐션: UMD 번들 동적 로드
                const mod = await loadExtensionBundle(ext);
                loadedRef.current.add(ext.id);

                if (mod?.registerSlots) {
                    mergeSlots(mod.registerSlots());
                }

                // i18n 로드
                const currentLang = i18n.language || 'en';
                await loadExtensionI18n(ext, currentLang);
                if (currentLang !== 'en') {
                    await loadExtensionI18n(ext, 'en');
                }
            }

            setSlots(newSlots);
            setLoading(false);
        };

        loadBundles();
    }, [extensions]);

    // 익스텐션 활성/비활성 토글
    const toggleExtension = useCallback(
        async (extId, enable) => {
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
                            msg = t('extensions.error_not_found', {
                                id: extId,
                                defaultValue: `Extension '${extId}' not found.`,
                            });
                            break;
                        case 'dependency_missing':
                            msg = t('extensions.error_dep_missing', {
                                id: extId,
                                dep: related,
                                defaultValue: `Cannot enable '${extId}': required extension '${related}' is not installed.`,
                            });
                            break;
                        case 'dependency_not_enabled':
                            msg = t('extensions.error_dep_disabled', {
                                id: extId,
                                dep: related,
                                defaultValue: `Cannot enable '${extId}': required extension '${related}' is not enabled. Enable it first.`,
                            });
                            break;
                        case 'has_dependents':
                            msg = t('extensions.error_has_dependents', {
                                id: extId,
                                deps: related,
                                defaultValue: `Cannot disable '${extId}': other extensions depend on it (${related}). Disable them first.`,
                            });
                            break;
                        case 'in_use':
                            msg = t('extensions.error_in_use', {
                                id: extId,
                                instances: related,
                                defaultValue: `Cannot disable '${extId}': in use by server instance(s): ${related}. Remove usage first.`,
                            });
                            break;
                        default:
                            msg =
                                result.error ||
                                t('extensions.error_unknown', { defaultValue: 'An unknown error occurred.' });
                    }

                    safeShowToast(msg, 'error', 5000);
                    console.warn(`[Extension] ${action} '${extId}' failed [${code}]:`, result.error);
                    return false;
                }

                await fetchExtensions(); // 목록 새로 가져오기
                return true;
            } catch (e) {
                const msg = t('extensions.error_network', {
                    action,
                    id: extId,
                    defaultValue: `Failed to ${action} extension '${extId}'. Check daemon connection.`,
                });
                safeShowToast(msg, 'error', 4000);
                console.warn(`[Extension] Failed to ${action} '${extId}':`, e);
                return false;
            }
        },
        [fetchExtensions, t],
    );

    // ── 레지스트리 페치 ──────────────────────────────────────────
    /** 원격 레지스트리에서 가용 익스텐션 목록을 가져옵니다. */
    const fetchRegistry = useCallback(async () => {
        setRegistryLoading(true);
        try {
            const data = await window.api.extensionFetchRegistry?.();
            if (data) {
                setRegistryExtensions(data.extensions || []);
                setAvailableUpdates(data.updates || []);
            }
            return data;
        } catch (e) {
            console.warn('[Extension] Failed to fetch registry:', e);
            return null;
        } finally {
            setRegistryLoading(false);
        }
    }, []);

    // ── 원클릭 설치 ──────────────────────────────────────────────
    /** 레지스트리에서 익스텐션을 다운로드·설치합니다. */
    const installExtension = useCallback(
        async (extId, opts = {}) => {
            if (!extId) return false;
            setInstallingIds((prev) => new Set(prev).add(extId));
            try {
                const result = await window.api.extensionInstall?.(extId, opts);
                if (result?.success === false) {
                    const msg = t('extensions.install_failed', {
                        id: extId,
                        error: result.error,
                        defaultValue: `Failed to install '${extId}': ${result.error}`,
                    });
                    safeShowToast(msg, 'error', 5000);
                    return false;
                }
                // 설치 후 목록 새로고침
                await fetchExtensions();
                await fetchRegistry();
                safeShowToast(
                    t('extensions.installed', { id: extId, defaultValue: `Extension '${extId}' installed.` }),
                    'success',
                    3000,
                );
                return true;
            } catch (e) {
                safeShowToast(
                    t('extensions.install_error', { id: extId, defaultValue: `Failed to install '${extId}'.` }),
                    'error',
                    4000,
                );
                console.warn(`[Extension] Failed to install '${extId}':`, e);
                return false;
            } finally {
                setInstallingIds((prev) => {
                    const n = new Set(prev);
                    n.delete(extId);
                    return n;
                });
            }
        },
        [fetchExtensions, fetchRegistry, t],
    );

    // ── 업데이트 체크 ────────────────────────────────────────────
    /** 설치된 익스텐션의 업데이트 가용 여부를 확인합니다. */
    const checkUpdates = useCallback(async () => {
        try {
            const data = await window.api.extensionCheckUpdates?.();
            if (data?.updates) {
                setAvailableUpdates(data.updates);
            }
            return data;
        } catch (e) {
            console.warn('[Extension] Failed to check updates:', e);
            return null;
        }
    }, []);

    // 익스텐션 제거
    const removeExtension = useCallback(
        async (extId) => {
            if (!extId) return false;
            try {
                const result = await window.api?.extensionRemove?.(extId);
                if (result?.success === false) {
                    const code = result.error_code || 'unknown';
                    const related = (result.related || []).join(', ');
                    let msg;
                    switch (code) {
                        case 'has_dependents':
                            msg = t('extensions.error_has_dependents', {
                                id: extId,
                                deps: related,
                                defaultValue: `Cannot remove '${extId}': other extensions depend on it (${related}).`,
                            });
                            break;
                        case 'in_use':
                            msg = t('extensions.error_in_use', {
                                id: extId,
                                instances: related,
                                defaultValue: `Cannot remove '${extId}': in use by server instance(s): ${related}.`,
                            });
                            break;
                        default:
                            msg =
                                result.error ||
                                t('extensions.error_unknown', { defaultValue: 'An unknown error occurred.' });
                    }
                    safeShowToast(msg, 'error', 5000);
                    return false;
                }
                await fetchExtensions();
                return true;
            } catch (_e) {
                safeShowToast(
                    t('extensions.error_network', {
                        action: 'remove',
                        id: extId,
                        defaultValue: `Failed to remove extension '${extId}'. Check daemon connection.`,
                    }),
                    'error',
                    4000,
                );
                return false;
            }
        },
        [fetchExtensions, t],
    );

    const value = {
        extensions,
        enabledExtensions,
        slots,
        loading,
        toggleExtension,
        refreshExtensions: fetchExtensions,
        removeExtension,
        // 레지스트리 & 버전관리
        registryExtensions,
        availableUpdates,
        registryLoading,
        installingIds,
        fetchRegistry,
        installExtension,
        checkUpdates,
    };

    return <ExtensionContext.Provider value={value}>{children}</ExtensionContext.Provider>;
}

export function useExtensions() {
    const ctx = useContext(ExtensionContext);
    if (!ctx) {
        // Context 없이 사용되는 경우 (단독 테스트 등) 기본값 반환
        return {
            extensions: [],
            enabledExtensions: [],
            slots: {},
            loading: false,
            toggleExtension: () => {},
            refreshExtensions: () => {},
            removeExtension: async () => false,
            registryExtensions: [],
            availableUpdates: [],
            registryLoading: false,
            installingIds: new Set(),
            fetchRegistry: async () => {},
            installExtension: async () => false,
            checkUpdates: async () => {},
        };
    }
    return ctx;
}

export default ExtensionContext;
