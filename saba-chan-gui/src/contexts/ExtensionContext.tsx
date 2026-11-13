/**
 * ExtensionContext — 범용 익스텐션 시스템 컨텍스트
 *
 * 역할:
 * 1. window.api.extensionList() (데몬 API) → 활성 익스텐션 목록
 * 2. 활성 익스텐션의 GUI 번들을 IPC를 통해 동적 로드
 * 3. 이름 규칙: window.SabaExt{Id} (예: id가 my-ext면 window.SabaExtMyExt)
 * 4. 슬롯 레지스트리 관리: slotId → [Component, ...]
 * 5. i18n 로드 + 기존 i18n에 병합
 */
import {
    type ComponentType,
    createContext,
    type ReactNode,
    useCallback,
    useContext,
    useEffect,
    useMemo,
    useRef,
    useState,
} from 'react';
import { useTranslation } from 'react-i18next';
import { forgetAssets } from '../extensions/host';
import { declaredSlots, loadsOn, type SlotMeta, type SlotSurface, slotProblems } from '../extensions/slotTable';
import { clearModuleAuditCache } from '../components/servers/ModuleAuditNotice';
import i18n from '../i18n';
import { useUIStore } from '../stores/useUIStore';
import type { ApiBody, ExtensionListItem } from '../types/daemon';
import { safeShowToast } from '../utils/helpers';
import { showTrustWarnings } from '../utils/trustNotice';
import { updateRequiredText } from '../utils/updateRequired';
import { failed } from '../utils/apiResult';
import { useInstallingIds } from '../components/Modals/useStoreInstall';

// 슬롯에 들어가는 컴포넌트: props는 슬롯마다 다르다
// biome-ignore lint/suspicious/noExplicitAny: 슬롯마다 컴포넌트 props가 다르다
type SlotComponent = ComponentType<any>;
/** slot에 들어간 한 칸: 넣은 익스텐션, 컴포넌트(설정 탭은 없으면 config_fields 자동 폼), manifest의 표시 정보 */
export interface SlotEntry {
    ext: string;
    Component?: SlotComponent;
    meta: SlotMeta;
}
/** slotId → 그 슬롯에 넣을 칸들 (익스텐션 목록 순, 한 익스텐션 안에서는 등록 순) */
export type SlotRegistry = Record<string, SlotEntry[]>;

/** 익스텐션 UMD 번들이 window.SabaExt{Id}에 거는 객체 */
interface ExtensionBundle {
    registerSlots?: () => Record<string, SlotComponent | SlotComponent[]>;
}

export interface ExtensionContextValue {
    extensions: ExtensionListItem[];
    slots: SlotRegistry;
    loading: boolean;
    toggleExtension: (extId: string, enable: boolean) => Promise<boolean> | undefined;
    refreshExtensions: () => Promise<boolean> | undefined;
    removeExtension: (extId: string) => Promise<boolean>;
    guiPending: ExtensionListItem[];
    approveExtensionGui: (extId: string) => Promise<boolean>;
    manifestExtensions: ApiBody[];
    availableUpdates: ApiBody[];
    manifestLoading: boolean;
    installingIds: Set<string>;
    fetchManifest: () => Promise<ApiBody | null | undefined>;
    installExtension: (extId: string, opts?: Record<string, unknown>) => Promise<boolean>;
}

const ExtensionContext = createContext<ExtensionContextValue | null>(null);

/** 데몬이 번들을 내주지 않았다: 확인되지 않았고 이 PC에서 허락하지 않음 (허락 뒤 파일이 바뀐 경우 포함) */
const GUI_UNAPPROVED = Symbol('extension_gui_unapproved');
// 번들을 받지 못했다(데몬 응답 없음 등): 로드됨으로 기억하지 않아 다음에 다시 받는다
const LOAD_FAILED = Symbol('load_failed');
// 오류 없이 실행된 번들이 남긴 전역 → 그 로드. 실패한 로드가 다른 로드의 전역을 자기 것으로 보지 않게 한다
const executedBy = new WeakMap<object, object>();
// 전역 이름 → 마지막으로 오류 없이 실행된 번들의 전역. 늦게 실행돼 덮어쓴 뒤 실패한 번들은 이 값으로 되돌린다
const lastExecuted = new Map<string, unknown>();

/**
 * 익스텐션 GUI 번들 동적 로드
 * IPC를 통해 JS 소스를 받아 blob URL로 <script> 태그 삽입
 * window.SabaExt{PascalId} 전역 객체를 등록함
 */
async function loadExtensionBundle(
    ext: ExtensionListItem,
): Promise<ExtensionBundle | typeof GUI_UNAPPROVED | typeof LOAD_FAILED | null> {
    const globalName: `SabaExt${string}` = `SabaExt${pascalCase(ext.id)}`;

    // 이미 로드됨
    if (window[globalName]) {
        return window[globalName] as ExtensionBundle;
    }

    // GUI manifest가 없는 익스텐션
    if (!ext.gui) {
        return null;
    }

    // 이 번들을 실행하기 직전의 전역과 이 번들이 남긴 전역. 실패하면 이 번들이 남긴 것만 되돌린다
    // (같은 확장을 동시에 읽은 다른 로드가 이미 성공해 등록한 전역은 지우지 않는다)
    let before: unknown;
    let mine: unknown;
    let executedBefore: unknown;
    const load = {};
    try {
        // IPC를 통해 번들 JS 소스 가져오기 (인증 포함)
        const jsSource = await window.api.extensionGuiBundle(ext.id);
        if ((jsSource as { error_code?: string } | null)?.error_code === 'extension_gui_unapproved') return GUI_UNAPPROVED;
        if (typeof jsSource !== 'string' || !jsSource) return LOAD_FAILED;

        // blob URL로 <script> 태그 삽입
        // 번들 실행 중 throw는 window error로 오고 onload도 불린다: 그 오류를 실패로 본다
        await new Promise<void>((resolve, reject) => {
            const blob = new Blob([jsSource], { type: 'application/javascript' });
            const url = URL.createObjectURL(blob);
            const script = document.createElement('script');
            let thrown: unknown = null;
            const onThrow = (ev: ErrorEvent) => {
                if (ev.filename !== url) return;
                thrown = ev.error || new Error(ev.message);
                // 실행 중 오류는 실행과 같은 차례에 온다: 이때의 전역이 이 번들이 남긴 것이다
                mine = window[globalName];
            };
            const done = () => {
                window.removeEventListener('error', onThrow);
                URL.revokeObjectURL(url);
            };
            window.addEventListener('error', onThrow);
            before = window[globalName];
            executedBefore = lastExecuted.get(globalName);
            script.src = url;
            script.async = true;
            script.onload = () => {
                done();
                if (thrown) return reject(thrown);
                // 실행 직후의 전역은 이 번들이 남긴 것이다 (onload는 실행과 같은 차례에 온다)
                const value = window[globalName];
                if (value && typeof value === 'object') {
                    executedBy.set(value, load);
                    lastExecuted.set(globalName, value);
                }
                resolve();
            };
            script.onerror = () => {
                done();
                reject(new Error(`Failed to execute bundle for '${ext.id}'`));
            };
            document.head.appendChild(script);
        });
        // 전역을 만들지 못한 번들도 실패다 (다음 목록에서 다시 읽는다)
        if (!window[globalName]) throw new Error(`Bundle for '${ext.id}' did not register ${globalName}`);
        mine = window[globalName];

        // CSS 로드 (있으면)
        const cssSource = await window.api.extensionGuiStyles(ext.id);
        if (cssSource) {
            const style = document.createElement('style');
            style.textContent = cssSource;
            style.dataset.extension = ext.id;
            document.head.appendChild(style);
        }

        return window[globalName] as ExtensionBundle;
    } catch (e) {
        console.warn(`[Extension] Failed to load GUI bundle for '${ext.id}':`, e);
        // 실행이 실패한 번들이 먼저 등록한 전역은 남기지 않는다 (다음 로드가 성공으로 보지 않게).
        // 그 뒤 다른 로드가 바꾼 전역은 그 로드의 것이라 두고, 실행 전에 있던 전역은 되살린다
        // 다른 로드가 오류 없이 실행해 남긴 전역이면 그 로드의 것이다 (이 번들이 등록 전에 throw한 경우)
        const owner = mine !== undefined && typeof mine === 'object' ? executedBy.get(mine as object) : undefined;
        if (mine !== undefined && window[globalName] === mine && (owner === undefined || owner === load)) {
            // 이 번들을 넣은 뒤 다른 로드가 오류 없이 실행했으면 그 전역으로 되돌린다
            const other = lastExecuted.get(globalName);
            const restore = other !== undefined && other !== mine && other !== executedBefore ? other : before;
            if (owner === load) lastExecuted.delete(globalName);
            if (restore === undefined) delete window[globalName];
            else window[globalName] = restore;
        }
        return LOAD_FAILED;
    }
}

function pascalCase(str: string) {
    return str.replace(/(^|[-_])(\w)/g, (_: string, __: string, c: string) => c.toUpperCase());
}

/**
 * 익스텐션 i18n 번역 로드 & i18next에 병합
 */
async function loadExtensionI18n(ext: ExtensionListItem, lang: string) {
    try {
        const translations = await window.api.extensionI18n(ext.id, lang);
        if (translations && typeof translations === 'object') {
            i18n.addResourceBundle(lang, `ext_${ext.id}`, translations, true, true);
        }
    } catch (e) {
        console.warn(`[Extension] Failed to load i18n for '${ext.id}' (${lang}):`, e);
    }
}

/**
 * 원격 데몬을 골랐으면 이 PC 데몬의 익스텐션 목록 (번들은 이 PC 데몬에서 받아 이 웹뷰에서 실행한다: 허락도 이 PC의 것을 본다).
 * 고르지 않았으면 null. 받지 못하면 throw한다: 빈 목록으로 보면 원격 PC의 허락으로 판단한 채 다시 묻지 않는다
 */
async function localExtensions(): Promise<ExtensionListItem[] | null> {
    const current = await window.api.daemonTarget?.();
    if (!current?.target) return null;
    const local = await window.api.extensionList({ local: true });
    if (!local || local.error || !Array.isArray(local.extensions)) {
        throw new Error(local?.error || 'local extension list unavailable');
    }
    return local.extensions;
}

/** 이 PC 데몬에서 허락을 기다리는 바탕화면·트레이 번들 (메인 slot도 함께 선언했어도. 그 창들은 이 PC 데몬을 본다) */
function localWindowPending(local: ExtensionListItem[]): ExtensionListItem[] {
    return local.filter(
        (ext) =>
            ext.enabled &&
            ext.gui?.bundle &&
            ext.gui_allowed === false &&
            (loadsOn(ext, 'desktop') || loadsOn(ext, 'tray')),
    );
}

/**
 * surface: 이 Provider가 놓인 창 (extensions/slotTable). 트레이 팝업은 그 창 slot을 선언한 번들만,
 * 이미 허락된 것만 읽고 허락을 묻지 않는다
 */
export function ExtensionProvider({ children, surface = 'main' }: { children: ReactNode; surface?: SlotSurface }) {
    const { t } = useTranslation();
    const [extensions, setExtensions] = useState<ExtensionListItem[]>([]);
    const [slots, setSlots] = useState<SlotRegistry>({});
    const [loading, setLoading] = useState(true);
    const loadedRef = useRef(new Set<string>());
    /** 로드된 익스텐션의 버전 추적 (핫로드 시 번들 무효화 판단용) */
    const loadedVersionsRef = useRef<Record<string, string>>({});
    /** 이 창에서 읽은 익스텐션 번역 (`id@version@lang`) */
    const i18nLoadedRef = useRef(new Set<string>());
    /** 이 창에서 번역을 쓰는 익스텐션 (언어를 바꾸면 그 언어로 다시 받는다) */
    const i18nExtsRef = useRef(new Map<string, ExtensionListItem>());
    /** 아직 읽지 않은 언어만 받는다. 새로 받은 것이 있으면 true */
    const loadI18nOnce = async (ext: ExtensionListItem, lang = i18n.language || 'en') => {
        i18nExtsRef.current.set(ext.id, ext);
        let loaded = false;
        for (const l of new Set([lang, 'en'])) {
            const key = `${ext.id}@${ext.version}@${l}`;
            if (i18nLoadedRef.current.has(key)) continue;
            i18nLoadedRef.current.add(key);
            await loadExtensionI18n(ext, l);
            loaded = true;
        }
        return loaded;
    };
    const loadI18nOnceRef = useRef(loadI18nOnce);
    loadI18nOnceRef.current = loadI18nOnce;
    useEffect(() => {
        let announcing = false;
        const onLanguage = async (lng: string) => {
            if (announcing) return;
            const exts = [...i18nExtsRef.current.values()].filter((ext) => enabledIdsRef.current.has(ext.id));
            const loaded = await Promise.all(exts.map((ext) => loadI18nOnceRef.current(ext, lng)));
            if (!loaded.includes(true) || i18n.language !== lng) return;
            // 화면은 언어가 바뀔 때만 다시 그려진다(번역이 늦게 들어온 것은 모른다): 받은 뒤 한 번 더 알린다
            announcing = true;
            i18n.emit('languageChanged', lng);
            announcing = false;
        };
        i18n.on('languageChanged', onLanguage);
        return () => i18n.off('languageChanged', onLanguage);
    }, []);
    /** 지금 켜진 확장 id (늦게 끝난 번들 읽기가 그 사이 꺼졌는지 본다) */
    const enabledIdsRef = useRef(new Set<string>());
    const unloadBundle = (id: string) => {
        document.querySelectorAll(`style[data-extension="${id}"]`).forEach((el) => {
            el.remove();
        });
        delete window[`SabaExt${pascalCase(id)}`];
        lastExecuted.delete(`SabaExt${pascalCase(id)}`);
        loadedRef.current.delete(id);
        delete loadedVersionsRef.current[id];
        forgetAssets(id);
    };
    /** GUI 번들을 허락받아야 하는 활성 익스텐션 (확인 안 됨, 이 PC에서 허락 안 함) */
    const [guiPending, setGuiPending] = useState<ExtensionListItem[]>([]);
    /** 원격 데몬을 고른 동안, 이 PC 데몬에서 허락을 기다리는 다른 창(바탕화면·트레이) 번들. 그 창들은 이 PC 데몬을 본다 */
    const [localPending, setLocalPending] = useState<ExtensionListItem[]>([]);
    /** 이번 실행에서 이미 물어본 익스텐션 (거절하면 다시 묻지 않고 설정 화면에서 허락하게 둔다) */
    const askedGuiRef = useRef(new Set<string>());
    const modalOpen = useUIStore((s) => Boolean(s.modal));

    // ── 익스텐션 스토어 (사바 스토리지 익스텐션 탭) ─────────────────
    // 화면을 떠났다 돌아와도 설치 중 표시와 받아 둔 매니페스트가 남도록 Provider에 둔다
    /** 원격 매니페스트에서 받아온 가용 익스텐션 목록 */
    const [manifestExtensions, setManifestExtensions] = useState<ApiBody[]>([]);
    /** 업데이트 가능한 익스텐션 목록 */
    const [availableUpdates, setAvailableUpdates] = useState<ApiBody[]>([]);
    const [manifestLoading, setManifestLoading] = useState(false);
    const { installingIds, run } = useInstallingIds();

    // 익스텐션 목록 가져오기
    // 받지 못하면 받아 둔 목록을 그대로 두고 false를 돌려준다
    const fetchExtensions = useCallback(async () => {
        try {
            const data = await window.api.extensionList(surface === 'desktop' ? { local: true } : undefined);
            if (!data || data.error || !Array.isArray(data.extensions)) {
                throw new Error(data?.error || 'extension list unavailable');
            }
            // 원격 데몬 목록의 gui_allowed는 그 PC의 허락이다: 이 PC에 있는 익스텐션은 이 PC의 허락으로 바꿔 둔다.
            // 이 PC 목록을 받지 못하면 받지 못한 것으로 본다 (false: 처음 로드는 다시 묻는다)
            const local = surface === 'desktop' ? null : await localExtensions();
            // 설치·업데이트·삭제·재검색 뒤 목록이 바뀌면 코드 검사도 다시 한다
            clearModuleAuditCache();
            const allowedHere = new Map(local?.map((ext) => [ext.id, ext.gui_allowed]));
            setExtensions(
                data.extensions.map((ext) =>
                    allowedHere.has(ext.id) ? { ...ext, gui_allowed: allowedHere.get(ext.id) } : ext,
                ),
            );
            if (surface === 'main') setLocalPending(localWindowPending(local ?? []));
            return true;
        } catch (e) {
            console.warn('[Extension] Failed to fetch extensions:', e);
            return false;
        }
    }, [surface]);

    // 초기 로드. 데몬이 아직 안 떴거나 잠깐 닿지 않으면 받을 때까지 간격을 늘려 다시 묻는다
    useEffect(() => {
        let alive = true;
        let timer: ReturnType<typeof setTimeout> | undefined;
        const attempt = async (delay: number): Promise<void> => {
            if ((await fetchExtensions()) || !alive) return;
            timer = setTimeout(() => attempt(Math.min(delay * 2, 30_000)), delay);
        };
        attempt(2000);
        return () => {
            alive = false;
            clearTimeout(timer);
        };
    }, [fetchExtensions]);

    // 바탕화면 오버레이(셸 overlay.rs)는 `Desktop.overlay`를 선언한 익스텐션이 켜져 있을 때만 뜬다.
    // 메인 창이 그 익스텐션을 켜고 끄거나 설치·삭제·허락하면 셸에 다시 보게 한다 (처음 목록은 셸이 데몬에 붙을 때 이미 봤다)
    const overlayKey = extensions
        .filter((ext) => ext.enabled && ext.gui?.bundle && Object.hasOwn(ext.gui?.slots ?? {}, 'Desktop.overlay'))
        .map((ext) => `${ext.id}:${ext.gui_allowed !== false}`)
        .join(',');
    const overlaySeen = useRef<string | null>(null);
    useEffect(() => {
        if (surface !== 'main' || loading) return;
        const first = overlaySeen.current === null;
        if (overlaySeen.current === overlayKey) return;
        overlaySeen.current = overlayKey;
        if (!first) window.api?.overlaySync?.();
    }, [surface, loading, overlayKey]);

    // 활성 익스텐션 변경 시 GUI 번들 로드 & 슬롯 등록
    useEffect(() => {
        const enabled = extensions.filter((ext) => ext.enabled);
        // 꺼졌거나 지운 확장은 번들을 내린다: 스타일이 다른 화면에 남지 않게, 다시 켜면 스타일까지 새로 읽게
        const enabledIds = new Set(enabled.map((ext) => ext.id));
        enabledIdsRef.current = enabledIds;
        for (const id of [...loadedRef.current]) {
            if (!enabledIds.has(id)) unloadBundle(id);
        }
        // 목록이 다시 바뀌면 이 로드의 결과는 버린다 (늦게 끝나 새 슬롯을 덮지 않게)
        let cancelled = false;

        const loadBundles = async () => {
            const newSlots: SlotRegistry = {};
            const pending: ExtensionListItem[] = [];

            /**
             * 번들이 등록한 컴포넌트를 manifest 선언과 맞춰 newSlots에 넣는다.
             * 선언과 어긋나면 경고만 남긴다. 컴포넌트 없이 선언만 한 설정 탭은 자동 폼 칸으로 넣는다
             */
            const mergeSlots = (ext: ExtensionListItem, extSlots?: Record<string, SlotComponent | SlotComponent[]>) => {
                const declared = declaredSlots(ext);
                const registered = extSlots || {};
                for (const problem of slotProblems(ext, Object.keys(registered))) {
                    console.warn(`[Extension] '${ext.id}': ${problem}`);
                }
                const push = (slotId: string, entry: SlotEntry) => {
                    if (!newSlots[slotId]) newSlots[slotId] = [];
                    newSlots[slotId].push(entry);
                };
                for (const [slotId, components] of Object.entries(registered)) {
                    for (const Component of Array.isArray(components) ? components : [components]) {
                        push(slotId, { ext: ext.id, Component, meta: declared[slotId] ?? {} });
                    }
                }
                if (surface === 'main' && declared['Settings.tab'] && !registered['Settings.tab']) {
                    push('Settings.tab', { ext: ext.id, meta: declared['Settings.tab'] });
                }
            };

            // 확장 하나가 throw해도 나머지 확장의 슬롯과 허락 대기 목록은 갱신한다
            const loadOne = async (ext: ExtensionListItem) => {
                const globalName: `SabaExt${string}` = `SabaExt${pascalCase(ext.id)}`;
                // 이 창에 둘 slot이 없는 익스텐션은 번들·번역을 읽지 않는다 (GUI 선언이 없는 익스텐션은 메인 창만).
                // 다른 창(트레이 팝업·바탕화면 오버레이)에만 그리는 번들도 허락은 메인 창이 묻는다
                if (!(ext.gui ? loadsOn(ext, surface) : surface === 'main')) {
                    if (surface === 'main' && ext.gui?.bundle && ext.gui_allowed === false) pending.push(ext);
                    return;
                }
                // 번들 없이 manifest 선언만 있는 익스텐션 (설정 탭 자동 폼 등)
                if (ext.gui && !ext.gui.bundle) {
                    mergeSlots(ext);
                    await loadI18nOnce(ext);
                    return;
                }
                const prevVersion = loadedVersionsRef.current[ext.id];
                const versionChanged = prevVersion && ext.version && prevVersion !== ext.version;

                // 버전 변경 감지 → 기존 번들 무효화 (핫로드)
                if (versionChanged && loadedRef.current.has(ext.id)) {
                    delete window[globalName];
                    loadedRef.current.delete(ext.id);
                    // 기존 <style> 태그 제거
                    document.querySelectorAll(`style[data-extension="${ext.id}"]`).forEach((el) => {
                        el.remove();
                    });
                    forgetAssets(ext.id);
                    console.info(`[Extension] Bundle invalidated for '${ext.id}' (${prevVersion} → ${ext.version})`);
                }

                // 로드됨으로 기억했는데 전역이 없다(겹친 로드의 실패 정리 등): 다시 읽는다
                if (ext.gui && loadedRef.current.has(ext.id) && !window[globalName]) {
                    loadedRef.current.delete(ext.id);
                    document.querySelectorAll(`style[data-extension="${ext.id}"]`).forEach((el) => {
                        el.remove();
                    });
                }

                // ① 이미 로드된 익스텐션 (버전 동일)
                if (loadedRef.current.has(ext.id)) {
                    const mod = window[globalName] as ExtensionBundle | undefined;
                    mergeSlots(ext, mod?.registerSlots?.());
                    return;
                }

                // 허락받지 않은 번들은 요청하지 않는다 (데몬이 gui_allowed로 알려 준다). 묻는 것은 메인 창만
                if (ext.gui && ext.gui_allowed === false) {
                    if (surface === 'main') pending.push(ext);
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
                    if (surface === 'main') pending.push(ext);
                    return;
                }
                if (mod === LOAD_FAILED) return;
                loadedRef.current.add(ext.id);
                loadedVersionsRef.current[ext.id] = ext.version;

                mergeSlots(ext, mod?.registerSlots?.());
                await loadI18nOnce(ext);
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
    }, [extensions, surface]);

    // 익스텐션 활성/비활성 토글
    const toggleExtension = useCallback(
        async (extId: string, enable: boolean) => {
            const action = enable ? 'enable' : 'disable';
            try {
                let result: ApiBody;
                if (enable) {
                    result = await window.api.extensionEnable(extId);
                } else {
                    result = await window.api.extensionDisable(extId);
                }

                if (result && failed(result)) {
                    // 구조화된 에러 응답 처리
                    const code = result.error_code || 'unknown';
                    const related = (result.related || []).join(', ');
                    let msg: string | undefined;

                    switch (code) {
                        // 사바쨩이 낮아 켤 수 없으면 업데이트가 필요하다고 알린다
                        case 'app_version_unsatisfied':
                        case 'component_version_unsatisfied':
                            msg = updateRequiredText(t, result, extId) || result.error;
                            break;
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
        async (extId: string) => {
            try {
                const res = await window.api.sandboxAllowExtensionGui(`ext-${extId}`);
                if (res?.error_code === 'canceled') return false;
                if (failed(res)) {
                    safeShowToast(t('trust.extension_gui.allow_failed', { error: res?.error || '' }), 'error', 5000);
                    return false;
                }
                safeShowToast(t('trust.extension_gui.allowed', { id: extId }), 'info', 4000);
                await fetchExtensions();
                // 이 PC의 바탕화면 번들이면 셸이 오버레이를 다시 보게 (원격 데몬을 고른 동안엔 위 목록에 없다)
                window.api?.overlaySync?.();
                return true;
            } catch (e) {
                safeShowToast(t('trust.extension_gui.allow_failed', { error: (e as Error).message }), 'error', 5000);
                return false;
            }
        },
        [fetchExtensions, t],
    );

    const allPending = useMemo(
        () => [...guiPending, ...localPending.filter((ext) => !guiPending.some((p) => p.id === ext.id))],
        [guiPending, localPending],
    );

    // 허락을 기다리는 익스텐션을 하나씩 묻는다. 다른 창이 떠 있으면 닫힌 뒤에 묻는다
    useEffect(() => {
        if (surface !== 'main' || modalOpen || !window.api?.sandboxAllowExtensionGui) return;
        const next = allPending.find((ext) => !askedGuiRef.current.has(ext.id));
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
    }, [surface, allPending, modalOpen, approveExtensionGui, t]);

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

    /** 매니페스트에서 익스텐션을 다운로드·설치합니다. 설치하면 목록과 매니페스트를 다시 받는다 */
    const installExtension = useCallback(
        async (extId: string, opts: Record<string, unknown> = {}) => {
            if (!extId) return false;
            return run(extId, async () => {
                try {
                    const result = await window.api.extensionInstall?.(extId, opts);
                    if (result && failed(result)) {
                        const msg =
                            // 함께 받는 의존 익스텐션이 거절되면 그 이름(result.extension)으로
                            updateRequiredText(t, result, result.extension || extId) ||
                            t('extensions.install_failed', {
                                id: extId,
                                error: result.error,
                                defaultValue: `Failed to install '${extId}': ${result.error}`,
                            });
                        safeShowToast(msg, 'error', 5000);
                        return false;
                    }
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
                }
            });
        },
        [fetchExtensions, fetchManifest, run, t],
    );

    // 익스텐션 제거
    const removeExtension = useCallback(
        async (extId: string) => {
            if (!extId) return false;
            try {
                const result = await window.api?.extensionRemove?.(extId);
                if (result && failed(result)) {
                    const code = result.error_code || 'unknown';
                    const related = (result.related || []).join(', ');
                    let msg: string | undefined;
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

    const value: ExtensionContextValue = {
        extensions,
        slots,
        loading,
        toggleExtension,
        refreshExtensions: fetchExtensions,
        removeExtension,
        // 확인되지 않은 익스텐션의 GUI 허락
        guiPending: allPending,
        approveExtensionGui,
        manifestExtensions,
        availableUpdates,
        manifestLoading,
        installingIds,
        fetchManifest,
        installExtension,
    };

    return <ExtensionContext.Provider value={value}>{children}</ExtensionContext.Provider>;
}

export function useExtensions(): ExtensionContextValue {
    const ctx = useContext(ExtensionContext);
    if (!ctx) {
        // Context 없이 사용되는 경우 (단독 테스트 등) 기본값 반환
        return {
            extensions: [],
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
            fetchManifest: async () => null,
            installExtension: async () => false,
        };
    }
    return ctx;
}

export default ExtensionContext;
