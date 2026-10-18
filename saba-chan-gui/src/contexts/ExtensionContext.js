/**
 * ExtensionContext — 범용 익스텐션 시스템 컨텍스트
 *
 * 역할:
 * 1. window.api.extensionList() (Electron IPC) → 활성 익스텐션 목록
 * 2. 활성 익스텐션의 GUI 번들을 IPC를 통해 동적 로드
 * 3. 이름 규칙: window.SabaExt{Id} (예: id가 my-ext면 window.SabaExtMyExt)
 * 4. 슬롯 레지스트리 관리: slotId → [Component, ...]
 * 5. i18n 로드 + 기존 i18n에 병합
 */
import { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { clearModuleAuditCache } from '../components/servers/ModuleAuditNotice';
import i18n from '../i18n';
import { useUIStore } from '../stores/useUIStore';
import { safeShowToast } from '../utils/helpers';
import { showTrustWarnings } from '../utils/trustNotice';

const ExtensionContext = createContext(null);

/** 데몬이 번들을 내주지 않았다: 확인되지 않았고 이 PC에서 허락하지 않음 (허락 뒤 파일이 바뀐 경우 포함) */
const GUI_UNAPPROVED = Symbol('extension_gui_unapproved');

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
        if (jsSource?.error_code === 'extension_gui_unapproved') return GUI_UNAPPROVED;
        if (typeof jsSource !== 'string' || !jsSource) return null;

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
    /** 로드된 익스텐션의 버전 추적 (핫로드 시 번들 무효화 판단용) */
    const loadedVersionsRef = useRef({});
    /** 지금 켜진 확장 id (늦게 끝난 번들 읽기가 그 사이 꺼졌는지 본다) */
    const enabledIdsRef = useRef(new Set());
    const unloadBundle = (id) => {
        document.querySelectorAll(`style[data-extension="${id}"]`).forEach((el) => el.remove());
        delete window[`SabaExt${pascalCase(id)}`];
        loadedRef.current.delete(id);
        delete loadedVersionsRef.current[id];
    };
    /** GUI 번들을 허락받아야 하는 활성 익스텐션 (확인 안 됨, 이 PC에서 허락 안 함) */
    const [guiPending, setGuiPending] = useState([]);
    /** 이번 실행에서 이미 물어본 익스텐션 (거절하면 다시 묻지 않고 설정 화면에서 허락하게 둔다) */
    const askedGuiRef = useRef(new Set());
    const modalOpen = useUIStore((s) => Boolean(s.modal));

    // ── 매니페스트 / 버전관리 상태 ──────────────────────────────
    /** 원격 매니페스트에서 받아온 가용 익스텐션 목록 */
    const [manifestExtensions, setManifestExtensions] = useState([]);
    /** 업데이트 가능한 익스텐션 목록 */
    const [availableUpdates, setAvailableUpdates] = useState([]);
    /** 매니페스트 페치 중 여부 */
    const [manifestLoading, setManifestLoading] = useState(false);
    /** 현재 설치 진행 중인 익스텐션 ID 집합 */
    const [installingIds, setInstallingIds] = useState(new Set());

    // 익스텐션 목록 가져오기
    const fetchExtensions = useCallback(async () => {
        try {
            const data = await window.api.extensionList();
            // 설치·업데이트·삭제·재검색 뒤 목록이 바뀌면 코드 검사도 다시 한다
            clearModuleAuditCache();
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
        // 꺼졌거나 지운 확장은 번들을 내린다: 스타일이 다른 화면에 남지 않게, 다시 켜면 스타일까지 새로 읽게
        const enabledIds = new Set(enabled.map((ext) => ext.id));
        enabledIdsRef.current = enabledIds;
        for (const id of [...loadedRef.current]) {
            if (!enabledIds.has(id)) unloadBundle(id);
        }
        // 목록이 다시 바뀌면 이 로드의 결과는 버린다 (늦게 끝나 새 슬롯을 덮지 않게)
        let cancelled = false;

        const loadBundles = async () => {
            const newSlots = {};
            const pending = [];

            /** 슬롯 맵을 newSlots에 병합하는 헬퍼 */
            const mergeSlots = (extSlots) => {
                for (const [slotId, components] of Object.entries(extSlots || {})) {
                    if (!newSlots[slotId]) newSlots[slotId] = [];
                    newSlots[slotId].push(...(Array.isArray(components) ? components : [components]));
                }
            };

            // 확장 하나가 throw해도 나머지 확장의 슬롯과 허락 대기 목록은 갱신한다
            const loadOne = async (ext) => {
                const globalName = `SabaExt${pascalCase(ext.id)}`;
                const prevVersion = loadedVersionsRef.current[ext.id];
                const versionChanged = prevVersion && ext.version && prevVersion !== ext.version;

                // 버전 변경 감지 → 기존 번들 무효화 (핫로드)
                if (versionChanged && loadedRef.current.has(ext.id)) {
                    delete window[globalName];
                    loadedRef.current.delete(ext.id);
                    // 기존 <style> 태그 제거
                    document.querySelectorAll(`style[data-extension="${ext.id}"]`).forEach((el) => el.remove());
                    console.info(`[Extension] Bundle invalidated for '${ext.id}' (${prevVersion} → ${ext.version})`);
                }

                // ① 이미 로드된 익스텐션 (버전 동일)
                if (loadedRef.current.has(ext.id)) {
                    const mod = window[globalName];
                    if (mod?.registerSlots) {
                        mergeSlots(mod.registerSlots());
                    }
                    return;
                }

                // 허락받지 않은 번들은 요청하지 않는다 (데몬이 gui_allowed로 알려 준다)
                if (ext.gui && ext.gui_allowed === false) {
                    pending.push(ext);
                    return;
                }

                // ② UMD 번들 동적 로드
                const mod = await loadExtensionBundle(ext);
                // 읽는 사이 꺼졌으면 넣은 스타일·전역을 내린다 (그 사이 돈 로드는 아직 이 번들을 몰랐다)
                if (!enabledIdsRef.current.has(ext.id)) {
                    unloadBundle(ext.id);
                    return;
                }
                if (mod === GUI_UNAPPROVED) {
                    pending.push(ext);
                    return;
                }
                loadedRef.current.add(ext.id);
                loadedVersionsRef.current[ext.id] = ext.version;

                if (mod?.registerSlots) {
                    mergeSlots(mod.registerSlots());
                }

                // i18n 로드
                const currentLang = i18n.language || 'en';
                await loadExtensionI18n(ext, currentLang);
                if (currentLang !== 'en') {
                    await loadExtensionI18n(ext, 'en');
                }
            };
            for (const ext of enabled) {
                try {
                    await loadOne(ext);
                } catch (e) {
                    console.warn(`[Extension] '${ext.id}' failed to load its GUI:`, e);
                }
            }

            if (cancelled) return;
            setSlots(newSlots);
            setGuiPending(pending);
            setLoading(false);
        };

        loadBundles();
        return () => {
            cancelled = true;
        };
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

                if (enable) showTrustWarnings(result, t);
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

    // 확인되지 않은 익스텐션의 GUI를 이 PC에서 허락한다 (데몬이 파일 지문과 함께 저장). 허락하면 다시 받아 로드한다
    const approveExtensionGui = useCallback(
        async (extId) => {
            try {
                const res = await window.api.sandboxAllowExtensionGui(`ext-${extId}`);
                if (res?.error_code === 'canceled') return false;
                if (!res || res.success === false || res.error) {
                    safeShowToast(t('trust.extension_gui.allow_failed', { error: res?.error || '' }), 'error', 5000);
                    return false;
                }
                safeShowToast(t('trust.extension_gui.allowed', { id: extId }), 'info', 4000);
                await fetchExtensions();
                return true;
            } catch (e) {
                safeShowToast(t('trust.extension_gui.allow_failed', { error: e.message }), 'error', 5000);
                return false;
            }
        },
        [fetchExtensions, t],
    );

    // 허락을 기다리는 익스텐션을 하나씩 묻는다. 다른 창이 떠 있으면 닫힌 뒤에 묻는다
    useEffect(() => {
        if (modalOpen || !window.api?.sandboxAllowExtensionGui) return;
        const next = guiPending.find((ext) => !askedGuiRef.current.has(ext.id));
        if (!next) return;
        askedGuiRef.current.add(next.id);
        const { openModal, closeModal } = useUIStore.getState();
        openModal({
            type: 'question',
            title: t('trust.extension_gui.title'),
            message: t('trust.extension_gui.message', { id: next.id }),
            buttons: [
                {
                    label: t('trust.extension_gui.allow'),
                    action: async () => {
                        closeModal();
                        await approveExtensionGui(next.id);
                    },
                },
                { label: t('trust.extension_gui.deny'), action: () => closeModal() },
            ],
        });
    }, [guiPending, modalOpen, approveExtensionGui, t]);

    // ── 매니페스트 페치 ──────────────────────────────────────────────
    /** 원격 매니페스트에서 가용 익스텐션 목록을 가져옵니다. */
    const fetchManifest = useCallback(async () => {
        setManifestLoading(true);
        try {
            const data = await window.api.extensionFetchManifest?.();
            if (data) {
                setManifestExtensions(data.extensions || []);
                setAvailableUpdates(data.updates || []);
            }
            return data;
        } catch (e) {
            console.warn('[Extension] Failed to fetch manifest:', e);
            return null;
        } finally {
            setManifestLoading(false);
        }
    }, []);

    // ── 원클릭 설치 ──────────────────────────────────────────────
    /** 매니페스트에서 익스텐션을 다운로드·설치합니다. */
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
                await fetchManifest();
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
        [fetchExtensions, fetchManifest, t],
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
        // 확인되지 않은 익스텐션의 GUI 허락
        guiPending,
        approveExtensionGui,
        // 매니페스트 & 버전관리
        manifestExtensions,
        availableUpdates,
        manifestLoading,
        installingIds,
        fetchManifest,
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
            guiPending: [],
            approveExtensionGui: async () => false,
            manifestExtensions: [],
            availableUpdates: [],
            manifestLoading: false,
            installingIds: new Set(),
            fetchManifest: async () => {},
            installExtension: async () => false,
            checkUpdates: async () => {},
        };
    }
    return ctx;
}

export default ExtensionContext;
