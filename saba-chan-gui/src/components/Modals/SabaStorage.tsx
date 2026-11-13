import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useExtensions } from '../../contexts/ExtensionContext';
import { ExtensionConfigForm } from '../ExtensionConfigForm';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { Disclosure } from '../ui/Disclosure';
import { ModuleMakerCard } from '../ModuleMakerCard';
import { useContextMenu } from '../ui/ContextMenu';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { type AddonCard, useAddonVersions } from './AddonVersions';
import { useAppVersion } from './AppVersion';
import { QuestionModal } from './Modals';
import { localizedDescription, localizedName } from '../../utils/localizedText';
import { type CatalogModule, unsupportedText } from '../../utils/moduleSupport';
import { formatBytes } from '../../utils/schedule';
import { clearModuleAuditCache, ExtensionAuditNotice } from '../servers/ModuleAuditNotice';
import { SabaEmpty, SabaFail } from '../ui/SabaArt';
import { TrustBadge } from '../ui/TrustBadge';
import { createTranslateError, safeShowToast } from '../../utils/helpers';
import type { ApiBody, ModuleInfo } from '../../types/daemon';
import { failed } from '../../utils/apiResult';
import { useInstallingIds } from './useStoreInstall';

// ─────────────────────────────────────────────────────────────
// 사바 스토리지 (구 업데이트 센터)
//
// 탭 3개:
//   [컴포넌트] — 기존 UpdatePanel 내용 그대로
//   [모듈]     — 설치된 모듈 + saba-chan-modules 레지스트리
//   [익스텐션] — 기존 익스텐션 탭 내용 이전
//
// @param {boolean}  devMode   - 개발자 모드
// @param {string}   initialTab - 처음 보일 탭 (components | modules | extensions)
// ─────────────────────────────────────────────────────────────
/** 브리지는 던지지 않고 실패 본문으로 알린다 */
function throwIfRejected(res: ApiBody | null | undefined) {
    if (failed(res)) throw new Error(res?.error || 'failed');
}

interface SabaStorageProps {
    devMode?: boolean;
    initialTab?: string;
}

function SabaStorage({ devMode, initialTab = 'components' }: SabaStorageProps) {
    const { t } = useTranslation('gui');
    const [activeTab, setActiveTab] = useState(initialTab);
    const tabsRef = useRef<HTMLDivElement>(null);
    const indicatorRef = useRef<HTMLDivElement>(null);
    const syncIndicator = useCallback(() => {
        const container = tabsRef.current;
        const indicator = indicatorRef.current;
        if (!container || !indicator) return;
        const activeBtn = container.querySelector<HTMLElement>('.saba-storage-tab.active');
        if (!activeBtn) return;
        indicator.style.left = `${activeBtn.offsetLeft}px`;
        indicator.style.width = `${activeBtn.offsetWidth}px`;
    }, []);
    // biome-ignore lint/correctness/useExhaustiveDependencies: activeTab triggers DOM re-render that syncIndicator reads via querySelector
    useEffect(() => {
        syncIndicator();
    }, [activeTab, syncIndicator]);

    return (
        // 설정의 한 탭으로 열린다: 제목(h3)은 설정 쪽이 다른 탭과 같은 모양으로 그린다
        <div className="saba-storage">
            {/* 내부 탭 */}
            <div className="saba-storage-tabs" ref={tabsRef}>
                <div className="saba-storage-tab-indicator" ref={indicatorRef} />
                {['components', 'modules', 'extensions'].map((tab) => (
                    <button
                        key={tab}
                        data-spot={`storage.tab.${tab}`}
                        className={clsx('saba-storage-tab', { active: activeTab === tab })}
                        onClick={() => setActiveTab(tab)}
                    >
                        {t(
                            `saba_storage.tab_${tab}`,
                            tab === 'components' ? '컴포넌트' : tab === 'modules' ? '모듈' : '익스텐션',
                        )}
                    </button>
                ))}
            </div>

            {/* 세 탭을 처음부터 모두 띄워 두고 숨기기만 한다: 탭을 바꿀 때마다 목록을 다시 받지 않는다 */}
            <div className="saba-storage-content">
                <div className="saba-storage-pane" hidden={activeTab !== 'components'}>
                    <ComponentsTab devMode={devMode} />
                </div>
                <div className="saba-storage-pane" hidden={activeTab !== 'modules'}>
                    <ModulesTab />
                </div>
                <div className="saba-storage-pane" hidden={activeTab !== 'extensions'}>
                    <ExtensionsTab />
                </div>
            </div>
        </div>
    );
}

// ─────────────────────────────────────────────────────────────
// 컴포넌트 탭 (기존 UpdatePanel 로직 그대로)
// ─────────────────────────────────────────────────────────────

// 컴포넌트 데이터 파싱 — 순수 함수이므로 컴포넌트 밖에 정의
// 모듈/익스텐션은 별도 탭이라 카드로 보이지 않지만(hidden), 모두 업데이트가 함께 받고 적용한다 (UP1).
// locales는 백그라운드 자동 업데이트 — 다루지 않는다
const COMPONENT_ICONS: Record<string, string> = {
    updater: 'componentUpdater',
    gui: 'componentGui',
    'saba-core': 'componentCore',
    cli: 'componentCli',
    installer: 'componentInstaller',
};

// 목록 순서 (열 때마다 같게). 여기 없는 것은 뒤에 이름순
const COMPONENT_ORDER = ['saba-core', 'gui', 'cli', 'updater', 'installer'];
const componentRank = (key: string) => {
    const i = COMPONENT_ORDER.indexOf(key);
    return i === -1 ? COMPONENT_ORDER.length : i;
};
const byName = (a: unknown, b: unknown) => String(a).localeCompare(String(b));

export const parseComponents = (comps?: ApiBody[] | null) =>
    (comps || [])
        .filter((c) => {
            const k = typeof c.component === 'string' ? c.component : String(c.component);
            return k !== 'locales';
        })
        .map((c) => {
            const key = typeof c.component === 'string' ? c.component : String(c.component);
            return {
                key,
                hidden: key.startsWith('module-') || key.startsWith('ext-'),
                display: c.display_name || key,
                icon: key.startsWith('module-') ? 'gamepad' : COMPONENT_ICONS[key] || 'server',
                current_version: c.current_version || '—',
                latest_version: c.latest_version || null,
                update_available: !!c.update_available,
                downloaded: !!c.downloaded,
                installed: !!c.installed,
                needsUpdater: key === 'gui' || key === 'saba-core',
            };
        })
        .sort((a, b) => componentRank(a.key) - componentRank(b.key) || byName(a.key, b.key));

// ─────────────────────────────────────────────────────────────
// 다운로드 진행률 링 (SVG)
// percent: 0–100 → 프로그레스 링,  -1 → indeterminate(스피너)
// ─────────────────────────────────────────────────────────────
const MOCK_RELAY_URL = 'http://127.0.0.1:9876';

// 업데이트 방식 → 데몬 업데이트 설정 값
const UPDATE_MODES: Record<string, { auto_download: boolean; auto_apply: boolean }> = {
    check: { auto_download: false, auto_apply: false },
    download: { auto_download: true, auto_apply: false },
    apply: { auto_download: true, auto_apply: true },
};

function DownloadRing({ percent }: { percent: number }) {
    const r = 13;
    const circumference = 2 * Math.PI * r;
    const indeterminate = percent < 0;
    const offset = indeterminate ? circumference * 0.75 : circumference * (1 - percent / 100);
    return (
        <div className="download-ring-wrapper" data-tooltip={indeterminate ? '...' : `${percent}%`}>
            <svg className="download-ring-svg" viewBox="0 0 32 32" width="28" height="28">
                <circle className="download-ring-bg" cx="16" cy="16" r={r}
                    fill="none" strokeWidth="3" />
                <circle
                    className={clsx('download-ring-fill', { indeterminate })}
                    cx="16" cy="16" r={r}
                    fill="none" strokeWidth="3"
                    strokeLinecap="round"
                    strokeDasharray={circumference}
                    strokeDashoffset={offset}
                    transform="rotate(-90 16 16)"
                />
            </svg>
            {!indeterminate && (
                <span className="download-ring-label">{percent}<small>%</small></span>
            )}
        </div>
    );
}

function ComponentsTab({ devMode }: { devMode?: boolean }) {
    const { t } = useTranslation('gui');

    const [components, setComponents] = useState<ReturnType<typeof parseComponents>>([]);
    const [checking, setChecking] = useState(false);
    const [busyKeys, setBusyKeys] = useState<Set<string>>(new Set());
    const [busyAll, setBusyAll] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [message, setMessage] = useState<string | null>(null);
    const [confirmRestart, setConfirmRestart] = useState(false);
    const [pendingRestartAction, setPendingRestartAction] = useState<(() => void | Promise<void>) | null>(null);
    const [mockMode, setMockMode] = useState<boolean | null>(null);
    // 데몬이 모의 릴레이 서명을 믿는 개발 빌드(--features mock-relay)인가. 아니면 MOCK으로 바꿔도 검증에서 막힌다
    const [mockRelayBuild, setMockRelayBuild] = useState(false);
    const [autoCheckEnabled, setAutoCheckEnabled] = useState(true);
    const [updateChannel, setUpdateChannel] = useState('stable');
    // 정식 채널인데 지금 판이 베타면 같은 판의 정식을 기다린다 (데몬 status.stable_wait). 알약에는 띄우지 않는다
    const [stableWait, setStableWait] = useState<string | null>(null);
    // 찾은 뒤 할 일: check(알리기만) / download(받아 두기) / apply(받고 다음 실행 때 적용). 적용은 데몬이 판단한다
    const [updateMode, setUpdateMode] = useState('check');
    const [downloadProgress, setDownloadProgress] = useState<{ component: string | null; percent: number; active: boolean }>({ component: null, percent: 0, active: false });
    const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
    const progressPollRef = useRef<ReturnType<typeof setInterval> | null>(null);
    // 진행률 폴러를 같이 쓰는 다운로드 수. 마지막 것이 끝날 때 멈춘다 (G-U8)
    const progressUsersRef = useRef(0);
    const mountedRef = useRef(true);
    const appVersion = useAppVersion(
        (key) => components.find((c) => c.key === key)?.display || key,
        components.find((c) => c.key === 'saba-core')?.current_version,
    );

    const markBusy = useCallback((key: string) => setBusyKeys((prev) => new Set(prev).add(key)), []);
    const clearBusy = useCallback(
        (key: string) =>
            setBusyKeys((prev) => {
                const n = new Set(prev);
                n.delete(key);
                return n;
            }),
        [],
    );

    // ── 다운로드 진행률 폴링 ─────────────────────────────────────────
    const startProgressPolling = useCallback(() => {
        progressUsersRef.current += 1;
        if (progressPollRef.current) return;
        progressPollRef.current = setInterval(async () => {
            try {
                const res = await window.api?.updaterDownloadProgress?.();
                if (res?.ok && mountedRef.current) {
                    const pct = res.total_bytes > 0
                        ? Math.round((res.bytes_received / res.total_bytes) * 100)
                        : -1; // -1 = indeterminate (총 크기 모름)
                    setDownloadProgress({ component: res.component, percent: pct, active: res.active });
                }
            } catch (_) { /* ignore */ }
        }, 250);
    }, []);
    const stopProgressPolling = useCallback(() => {
        progressUsersRef.current = Math.max(0, progressUsersRef.current - 1);
        if (progressUsersRef.current > 0) return;
        if (progressPollRef.current) {
            clearInterval(progressPollRef.current);
            progressPollRef.current = null;
        }
        setDownloadProgress({ component: null, percent: 0, active: false });
    }, []);

    const handleToggleMock = useCallback(async (toMock: boolean) => {
        setError(null);
        setMessage(null);
        try {
            if (toMock) {
                // 업데이트는 릴레이 전용이다. 데몬은 relay_url만 본다 (src/ipc/updates.rs set_config)
                throwIfRejected(await window.api?.updaterSetConfig?.({ relay_url: MOCK_RELAY_URL }));
                setMockMode(true);
                setMessage(t('saba_storage.switch_to_mock'));
            } else {
                // 빈 문자열이면 데몬이 기본 릴레이로 돌아간다
                throwIfRejected(await window.api?.updaterSetConfig?.({ relay_url: '' }));
                setMockMode(false);
                setMessage(t('saba_storage.switch_to_real'));
            }
            setComponents([]);
        } catch (e) {
            setError(t('saba_storage.switch_failed', { error: (e as Error).message }));
        }
    }, [t]);

    const refreshStatus = useCallback(async () => {
        try {
            const res = await window.api?.updaterStatus?.();
            if (res?.ok && mountedRef.current) {
                setComponents(parseComponents(res.components));
                setStableWait(res.stable_wait || null);
            }
        } catch (_) {}
    }, []);

    // 수동 새로고침 — 데몬이 릴레이(/v1/updates/check)에 묻는다 (사용자 명시적 요청 시에만)
    const handleCheck = useCallback(async () => {
        setChecking(true);
        setError(null);
        setMessage(null);
        try {
            const res = await window.api?.updaterCheck?.();
            if (res?.ok) {
                setComponents(parseComponents(res.components));
            } else {
                setError(res?.error || 'Unknown error');
            }
        } catch (e) {
            setError((e as Error).message);
        }
        setChecking(false);
    }, []);

    const handleDownloadOne = useCallback(
        async (key: string) => {
            markBusy(key);
            setError(null);
            startProgressPolling();
            try {
                const res = await window.api?.updaterDownload?.([key]);
                if (!res?.ok) setError(`${key}: ${res?.error || 'Download failed'}`);
                await refreshStatus();
            } catch (e) {
                setError(`${key}: ${(e as Error).message}`);
            }
            stopProgressPolling();
            clearBusy(key);
        },
        [refreshStatus, markBusy, clearBusy, startProgressPolling, stopProgressPolling],
    );

    const handleApplyOne = useCallback(
        async (key: string) => {
            if (key === 'gui' || key === 'saba-core') {
                setPendingRestartAction(() => async () => {
                    markBusy(key);
                    setError(null);
                    setMessage(null);
                    try {
                        const res = await window.api?.updaterApply?.([key]);
                        if (res?.requires_updater && res?.needs_updater?.length > 0) {
                            const launchRes = await window.api?.updaterLaunchApply?.(res.needs_updater);
                            if (launchRes?.ok === false)
                                setError(`${key}: ${launchRes?.error || t('saba_storage.updater_launch_failed')}`);
                        } else if (failed(res)) {
                            setError(
                                `${key}: ${res?.errors?.length > 0 ? res.errors.join('; ') : res?.error || t('saba_storage.apply_failed')}`,
                            );
                        }
                    } catch (e) {
                        setError(`${key}: ${(e as Error).message}`);
                    }
                    clearBusy(key);
                });
                setConfirmRestart(true);
                return;
            }
            markBusy(key);
            setError(null);
            setMessage(null);
            try {
                const res = await window.api?.updaterApply?.([key]);
                clearModuleAuditCache();
                if (failed(res)) {
                    setError(`${key}: ${res?.errors?.length > 0 ? res.errors.join('; ') : res?.error || t('saba_storage.apply_failed')}`);
                } else {
                    if (res?.requires_updater && res?.needs_updater?.length > 0) {
                        const launchRes = await window.api?.updaterLaunchApply?.(res.needs_updater);
                        if (launchRes?.ok === false) setError(`${key}: ${launchRes?.error || t('saba_storage.updater_launch_failed')}`);
                        else setMessage(t('saba_storage.updater_background', { key }));
                    } else if (res?.applied?.length > 0) {
                        setMessage(t('saba_storage.apply_completed', { names: res.applied.join(', ') }));
                    } else {
                        setMessage(t('saba_storage.apply_request_done', { key }));
                    }
                }
                await refreshStatus();
            } catch (e) {
                setError(`${key}: ${(e as Error).message}`);
            }
            clearBusy(key);
        },
        [refreshStatus, markBusy, clearBusy, t],
    );

    const executeUpdateAll = useCallback(async () => {
        const updatable = components.filter((c) => c.update_available);
        if (updatable.length === 0) return;
        let hadError = false;
        setBusyAll(true);
        setError(null);
        setMessage(null);
        const allKeys = updatable.map((c) => c.key);
        for (const key of allKeys) markBusy(key);
        startProgressPolling();
        for (const key of allKeys) {
            try {
                const res = await window.api?.updaterDownload?.([key]);
                if (!res?.ok) {
                    hadError = true;
                    setError((prev) => (prev ? prev + '\n' : '') + `${key}: ${res?.error}`);
                }
            } catch (e) {
                hadError = true;
                setError((prev) => (prev ? prev + '\n' : '') + `${key}: ${(e as Error).message}`);
            }
            clearBusy(key);
        }
        stopProgressPolling();
        await refreshStatus();

        // 데몬에 모든 컴포넌트를 보내고, 데몬이 라우팅을 결정하도록 한다.
        // 업데이터가 대상이면 데몬이 전부 needs_updater로 보내고,
        // 아니면 데몬이 직접 적용하고 GUI/Updater만 needs_updater로 보낸다.
        let daemonApplied: string[] = [];
        for (const key of allKeys) markBusy(key);
        try {
            const res = await window.api?.updaterApply?.(allKeys);
            clearModuleAuditCache();
            // 일부만 적용돼도(ok:false) 적용된 것은 알린다 (G-U9b)
            daemonApplied = res?.applied || [];
            if (failed(res)) {
                hadError = true;
                setError(
                    (prev) =>
                        (prev ? prev + '\n' : '') +
                        (res?.errors?.length > 0 ? res.errors.join('; ') : res?.error || t('saba_storage.apply_failed')),
                );
            }
            // 데몬이 needs_updater로 보낸 컴포넌트가 있으면 업데이터 exe 스폰
            if (res?.requires_updater && res?.needs_updater?.length > 0) {
                const launchRes = await window.api?.updaterLaunchApply?.(res.needs_updater);
                if (launchRes?.ok === false) {
                    hadError = true;
                    setError((prev) => (prev ? prev + '\n' : '') + (launchRes?.error || t('saba_storage.updater_launch_failed')));
                } else {
                    // 업데이터 실행 → 데몬이 내려가므로 더 이상 진행하지 않음 (이후 API 호출 불가).
                    // GUI가 바뀌면 GUI도 곧 종료되고, 아니면 GUI는 남아 다시 뜬 데몬에 붙는다
                    for (const key of allKeys) clearBusy(key);
                    setBusyAll(false);
                    if (launchRes?.gui_restart === false)
                        setMessage(t('saba_storage.updater_background', { key: res.needs_updater.join(', ') }));
                    return;
                }
            }
        } catch (e) {
            hadError = true;
            setError((prev) => (prev ? prev + '\n' : '') + (e as Error).message);
        }
        for (const key of allKeys) clearBusy(key);
        await refreshStatus();
        if (daemonApplied.length > 0) setMessage(t('saba_storage.apply_completed', { names: daemonApplied.join(', ') }));
        else if (!hadError) setMessage(t('saba_storage.update_done'));
        setBusyAll(false);
    }, [components, refreshStatus, markBusy, clearBusy, startProgressPolling, stopProgressPolling, t]);

    const handleUpdateAll = useCallback(() => {
        const updatable = components.filter((c) => c.update_available);
        if (updatable.length === 0) return;
        const hasNeedsUpdater = updatable.some((c) => c.key === 'gui' || c.key === 'saba-core');
        if (hasNeedsUpdater) {
            setPendingRestartAction(() => () => executeUpdateAll());
            setConfirmRestart(true);
        } else {
            executeUpdateAll();
        }
    }, [components, executeUpdateAll]);

    const handleConfirmRestart = useCallback(() => {
        setConfirmRestart(false);
        if (pendingRestartAction) {
            pendingRestartAction();
            setPendingRestartAction(null);
        }
    }, [pendingRestartAction]);

    const handleAutoCheckToggle = useCallback(async (enabled: boolean) => {
        setAutoCheckEnabled(enabled);
        try {
            throwIfRejected(await window.api?.updaterSetConfig?.({ enabled }));
        } catch (e) {
            console.error('Failed to set auto-check config:', e);
            setAutoCheckEnabled(!enabled);
            setError(t('saba_storage.switch_failed', { error: (e as Error).message }));
        }
    }, [t]);

    const handleUpdateModeChange = useCallback(
        async (mode: string) => {
            const previous = updateMode;
            if (mode === previous) return;
            setUpdateMode(mode);
            try {
                throwIfRejected(await window.api?.updaterSetConfig?.(UPDATE_MODES[mode]));
            } catch (e) {
                console.error('Failed to set update mode:', e);
                setUpdateMode(previous);
                setError(t('saba_storage.switch_failed', { error: (e as Error).message }));
            }
        },
        [t, updateMode],
    );

    // 위 채널은 아래 채널 릴리즈도 받는다 (정식 ⊂ 미리보기 ⊂ 베타). 고를 수 있는 값은 데몬이 다시 확인한다
    const handleChannelChange = useCallback(
        async (channel: string) => {
            const previous = updateChannel;
            if (channel === previous) return;
            setUpdateChannel(channel);
            try {
                throwIfRejected(await window.api?.updaterSetConfig?.({ channel }));
            } catch (e) {
                console.error('Failed to set update channel:', e);
                setUpdateChannel(previous);
                setError(t('saba_storage.switch_failed', { error: (e as Error).message }));
                return;
            }
            // 받을 수 있는 버전이 채널마다 다르다: 바꾼 채널로 다시 묻는다
            await handleCheck();
        },
        [t, updateChannel, handleCheck],
    );

    useEffect(() => {
        mountedRef.current = true;
        (async () => {
            try {
                const cfgRes = await window.api?.updaterGetConfig?.();
                const cfg = cfgRes?.config || cfgRes;
                const isMock = !!cfg?.relay_url?.includes('127.0.0.1');
                if (mountedRef.current) {
                    setMockMode(isMock);
                    setMockRelayBuild(cfgRes?.mock_relay === true);
                    setAutoCheckEnabled(cfg?.enabled !== false);
                    if (cfg?.channel) setUpdateChannel(cfg.channel);
                    setUpdateMode(cfg?.auto_apply ? 'apply' : cfg?.auto_download ? 'download' : 'check');
                }
            } catch (_) {
                if (mountedRef.current) setMockMode(false);
            }
        })();

        // 마운트 시: 데몬 캐시 읽기 → 비어있으면 한 번만 check 실행
        (async () => {
            try {
                const res = await window.api?.updaterStatus?.();
                if (res?.ok && mountedRef.current) setStableWait(res.stable_wait || null);
                if (res?.ok && res.components?.length && mountedRef.current) {
                    setComponents(parseComponents(res.components));
                } else if (mountedRef.current) {
                    // 캐시가 비어있음 (아직 check가 한 번도 안 됨) → 최초 1회 check
                    setChecking(true);
                    const checkRes = await window.api?.updaterCheck?.();
                    if (checkRes?.ok && mountedRef.current) {
                        setComponents(parseComponents(checkRes.components));
                    }
                    if (mountedRef.current) setChecking(false);
                }
            } catch (_) {
                if (mountedRef.current) setChecking(false);
            }
        })();

        // 30초마다 캐시 폴링
        pollRef.current = setInterval(async () => {
            try {
                const res = await window.api?.updaterStatus?.();
                if (res?.ok && mountedRef.current) {
                    setComponents(parseComponents(res.components));
                    setStableWait(res.stable_wait || null);
                }
            } catch (_) {}
        }, 30000);

        return () => {
            mountedRef.current = false;
            clearInterval(pollRef.current!);
            if (progressPollRef.current) clearInterval(progressPollRef.current);
        };
        // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only initialization — async tasks use refs for mount guard
    }, []);

    const updatable = components.filter((c) => c.update_available);
    const allUpToDate = components.length > 0 && updatable.length === 0;
    const anyBusy = checking || busyAll || busyKeys.size > 0;

    return (
        <>
            <div className="saba-storage-tab-content">
                {/* 모의 릴레이 빌드에서만 보인다. 일반 빌드인데 이미 MOCK이면 REAL로 되돌릴 수 있게 남긴다 */}
                {devMode && (mockRelayBuild || mockMode) && (
                    <div className="update-panel-header" style={{ paddingTop: 0, paddingBottom: '8px' }}>
                        <label
                            className="update-mock-toggle"
                            data-tooltip={mockMode ? t('saba_storage.mock_server_label') : t('saba_storage.real_api_label')}
                        >
                            <span className={clsx('update-mock-label', mockMode ? 'mock' : 'real')}>
                                {mockMode ? 'MOCK' : 'REAL'}
                            </span>
                            <SabaToggle
                                size="sm"
                                checked={!!mockMode}
                                onChange={(checked) => handleToggleMock(checked)}
                                disabled={mockMode === null || anyBusy}
                            />
                        </label>
                    </div>
                )}

                {checking && (
                    <div className="update-modal-status-bar">
                        <Icon name="loader" size="sm" /> {t('saba_storage.checking')}
                    </div>
                )}
                {error && (
                    <div className="update-modal-status-bar error">
                        <Icon name="xCircle" size="sm" /> {error}
                    </div>
                )}
                {message && (
                    <div className="update-modal-status-bar success">
                        <Icon name="checkCircle" size="sm" /> {message}
                    </div>
                )}
                {appVersion.banner}

                <div className="ss-store-header">
                    <div className="ss-section-label" style={{ margin: 0 }}>
                        {t('saba_storage.tab_components', '컴포넌트')}
                    </div>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                        {updatable.length > 0 && (
                            <button
                                className="ss-icon-btn accent"
                                disabled={anyBusy}
                                onClick={handleUpdateAll}
                                data-tooltip={t('saba_storage.update_all', '모두 업데이트')}
                                aria-label={t('saba_storage.update_all', '모두 업데이트')}
                            >
                                <Icon name="download" size="sm" />
                            </button>
                        )}
                        <button
                            className="ss-icon-btn"
                            disabled={anyBusy}
                            onClick={handleCheck}
                            data-tooltip={t('saba_storage.refresh', '새로고침')}
                            aria-label={t('saba_storage.refresh', '새로고침')}
                        >
                            <Icon name={checking ? 'loader' : 'refresh'} size="sm" />
                        </button>
                    </div>
                </div>

                <div className="ss-cards">
                    {components.filter((c) => !c.hidden).map((c) => {
                        const isBusy = busyKeys.has(c.key) || busyAll;
                        const isDownloading = isBusy && downloadProgress.active && downloadProgress.component === c.key;
                        const pct = downloadProgress.percent;
                        return (
                            <div key={c.key} className="ss-card">
                                <div className="ss-card-icon">
                                    <Icon name={c.icon} size="md" />
                                </div>
                                <div className="ss-card-body">
                                    <span className="ss-card-name">
                                        {c.display}
                                        {c.update_available && c.latest_version && (
                                            <span className="ss-update-badge">v{c.latest_version}</span>
                                        )}
                                    </span>
                                    <span className="ss-card-version">
                                        {appVersion.was(c.current_version)}v{c.current_version}
                                    </span>
                                </div>
                                <div className="ss-card-actions">
                                    {c.update_available && !c.downloaded && (
                                        <>
                                            {isDownloading ? (
                                                <DownloadRing percent={pct} />
                                            ) : (
                                                <button
                                                    className="ss-icon-btn accent"
                                                    disabled={isBusy}
                                                    onClick={() => handleDownloadOne(c.key)}
                                                    data-tooltip={t('saba_storage.download')}
                                                    aria-label={t('saba_storage.download')}
                                                >
                                                    {isBusy ? <SabaSpinner size="xs" /> : <Icon name="download" size="sm" />}
                                                </button>
                                            )}
                                        </>
                                    )}
                                    {c.update_available && c.downloaded && (
                                        <button
                                            className={clsx('ss-icon-btn', c.needsUpdater ? 'warning' : 'accent')}
                                            disabled={isBusy}
                                            onClick={() => handleApplyOne(c.key)}
                                            data-tooltip={t(c.needsUpdater ? 'saba_storage.apply_needs_restart' : 'saba_storage.apply')}
                                            aria-label={t(c.needsUpdater ? 'saba_storage.apply_needs_restart' : 'saba_storage.apply')}
                                        >
                                            {isBusy ? (
                                                <SabaSpinner size="xs" />
                                            ) : (
                                                <Icon
                                                    name={c.needsUpdater ? 'externalLink' : 'checkCircle'}
                                                    size="sm"
                                                />
                                            )}
                                        </button>
                                    )}
                                    {!c.update_available && (
                                        <span className="ss-status-ok">
                                            <Icon name="checkCircle" size="sm" />
                                        </span>
                                    )}
                                </div>
                            </div>
                        );
                    })}
                </div>

                {allUpToDate && !message && (
                    <div className="update-modal-status success">
                        <Icon name="checkCircle" size="sm" />{' '}
                        {t('updates.no_updates', '모든 컴포넌트가 최신 버전입니다.')}
                    </div>
                )}
            </div>

            <div className="update-panel-settings">
                <label className="update-panel-setting-row">
                    <span className="update-panel-setting-label">
                        <Icon name="refresh" size="sm" />
                        {t('updates.auto_check')}
                    </span>
                    <SabaToggle checked={autoCheckEnabled} onChange={(checked) => handleAutoCheckToggle(checked)} />
                </label>
                {autoCheckEnabled && (
                    <div className="update-panel-setting-row">
                        <span className="update-panel-setting-label">
                            <Icon name="download" size="sm" />
                            {t('updates.mode')}
                        </span>
                        <CustomDropdown
                            className="update-channel-select"
                            ariaLabel={t('updates.mode')}
                            value={updateMode}
                            onChange={handleUpdateModeChange}
                            options={Object.keys(UPDATE_MODES).map((value) => ({
                                value,
                                label: t(`updates.mode_${value}`),
                            }))}
                        />
                    </div>
                )}
                <div className="update-panel-setting-row">
                    <span className="update-panel-setting-label">
                        <Icon name="antenna" size="sm" />
                        {t('updates.channel')}
                    </span>
                    <CustomDropdown
                        className="update-channel-select"
                        ariaLabel={t('updates.channel')}
                        value={updateChannel}
                        onChange={handleChannelChange}
                        options={['stable', 'rc', 'beta'].map((value) => ({
                            value,
                            label: t(`updates.channel_${value}`),
                        }))}
                    />
                </div>
                {stableWait && <p className="setting-description update-stable-wait">{t('updates.stable_wait', { version: stableWait })}</p>}
                {appVersion.row}
            </div>

            <div className="update-panel-actions" />

            {appVersion.dialogs}
            {confirmRestart && (
                <QuestionModal
                    title={t('saba_storage.restart_required_title')}
                    message={t('saba_storage.restart_required_message')}
                    onConfirm={handleConfirmRestart}
                    onCancel={() => {
                        setConfirmRestart(false);
                        setPendingRestartAction(null);
                    }}
                />
            )}
        </>
    );
}

// ─────────────────────────────────────────────────────────────
// 모듈 카드 아이콘 (icon.png 로드 시도, 실패 시 package 아이콘)
// ─────────────────────────────────────────────────────────────
function ModuleCardIcon({ icon }: { icon?: string | null }) {
    if (!icon) {
        return (
            <div className="ss-card-icon">
                <Icon name="package" size="md" />
            </div>
        );
    }
    return (
        <div className="ss-card-icon">
            <img src={icon} alt="" />
        </div>
    );
}

// ─────────────────────────────────────────────────────────────
// 모듈 탭
// ─────────────────────────────────────────────────────────────
function ModulesTab() {
    const { t, i18n } = useTranslation('gui');
    const openMenu = useContextMenu();
    const { refreshExtensions } = useExtensions();
    const [installedModules, setInstalledModules] = useState<ModuleInfo[] | null>(null); // null = 받는 중
    const [manifestModules, setManifestModules] = useState<CatalogModule[] | null>(null); // null = 미로드, [] = 로드됨
    const [manifestLoading, setManifestLoading] = useState(false);
    const [manifestError, setManifestError] = useState<string | null>(null);
    const { installingIds, run: runInstall } = useInstallingIds();
    const [installResults, setInstallResults] = useState<Record<string, string | null>>({}); // { id: 'ok'|'error' }
    const [removingIds, setRemovingIds] = useState<Set<string>>(new Set());
    const [confirmRemoveId, setConfirmRemoveId] = useState<string | null>(null);
    const [refreshingModules, setRefreshingModules] = useState(false);

    // 설치된 모듈 로드
    const loadInstalledModules = useCallback(async () => {
        try {
            const res = await window.api?.moduleList?.();
            if (res?.modules) {
                clearModuleAuditCache();
                setInstalledModules(res.modules);
                return;
            }
        } catch (_) {}
        // 받지 못하면 빈 목록으로 (계속 받는 중으로 두지 않는다)
        setInstalledModules((prev) => prev ?? []);
    }, []);

    // 모듈 캐시 새로고침 (디스크 재스캔)
    const handleRefreshModules = useCallback(async () => {
        setRefreshingModules(true);
        try {
            const res = await window.api?.moduleRefresh?.();
            if (res?.modules) {
                clearModuleAuditCache();
                setInstalledModules(res.modules);
            }
        } catch (_) {}
        setRefreshingModules(false);
    }, []);

    useEffect(() => {
        loadInstalledModules();
    }, [loadInstalledModules]);

    const versions = useAddonVersions(loadInstalledModules);

    // 받을 수 있는 목록은 탭을 열면 바로 받는다 (예전처럼 ▾ 뒤에 접어 두면 지나치기 쉽다)
    const loadManifest = useCallback(async () => {
        setManifestModules(null);
        setManifestLoading(true);
        setManifestError(null);
        try {
            const res = await window.api?.moduleManifest?.();
            if (res?.ok && res.manifest?.modules) {
                // manifest.modules: { id: { version, display_name, display_names?, descriptions?, platform_supported, ... } }
                const mods = Object.entries(res.manifest.modules as Record<string, Omit<CatalogModule, 'id'>>).map(([id, info]) => ({ ...info, id }));
                setManifestModules(mods);
            } else {
                setManifestError(
                    res?.error || t('saba_storage.manifest_fetch_failed', '매니페스트를 가져오지 못했습니다.'),
                );
                setManifestModules([]);
            }
        } catch (e) {
            setManifestError((e as Error).message);
            setManifestModules([]);
        }
        setManifestLoading(false);
    }, [t]);

    useEffect(() => {
        loadManifest();
    }, [loadManifest]);

    const handleInstallModule = useCallback(async (moduleId: string) => runInstall(moduleId, async () => {
        setInstallResults((prev) => ({ ...prev, [moduleId]: null }));
        try {
            const res = await window.api?.moduleInstallFromManifest?.(moduleId);
            if (res?.ok) {
                setInstallResults((prev) => ({ ...prev, [moduleId]: 'ok' }));
                // 데몬이 이 모듈이 쓰는 익스텐션을 받고 켰다: 알리고 익스텐션 목록도 새로
                const ext = res.extensions;
                if (ext?.enabled?.length) {
                    safeShowToast(t('saba_storage.extensions_enabled', { ids: ext.enabled.join(', ') }), 'info', 4000);
                }
                for (const f of ext?.failed || []) {
                    const error = createTranslateError(t)(f.error);
                    safeShowToast(t('saba_storage.extension_failed', { id: f.id, error }), 'warning', 6000);
                }
                if (ext?.installed?.length || ext?.enabled?.length) refreshExtensions?.();
                // 설치 후 로컬 모듈 목록 갱신
                clearModuleAuditCache();
                const listRes = await window.api?.moduleRefresh?.();
                if (listRes?.modules) setInstalledModules(listRes.modules);
            } else if (res?.error_code === 'module_busy') {
                // 같은 모듈의 다른 설치·제거가 끝나지 않았다: 실패로 굳히지 않고 다시 누를 수 있게 둔다
                safeShowToast(t('saba_storage.module_busy'), 'warning', 4000);
            } else {
                setInstallResults((prev) => ({ ...prev, [moduleId]: 'error' }));
            }
        } catch (_) {
            setInstallResults((prev) => ({ ...prev, [moduleId]: 'error' }));
        }
    }), [t, refreshExtensions, runInstall]);

    const handleConfirmRemove = useCallback(async () => {
        const id = confirmRemoveId;
        setConfirmRemoveId(null);
        if (!id) return;
        setRemovingIds((prev) => new Set(prev).add(id));
        try {
            const res = await window.api?.moduleRemove?.(id);
            if (res?.ok) await loadInstalledModules();
            else if (res?.error_code === 'module_busy') safeShowToast(t('saba_storage.module_busy'), 'warning', 4000);
            else if (res?.error_code === 'module_in_use') safeShowToast(t('saba_storage.module_in_use'), 'warning', 5000);
            else safeShowToast(t('saba_storage.remove_failed', { error: createTranslateError(t)(res?.error) }), 'error', 5000);
        } catch (e) {
            safeShowToast(t('saba_storage.remove_failed', { error: createTranslateError(t)((e as Error)?.message) }), 'error', 5000);
        }
        setRemovingIds((prev) => {
            const n = new Set(prev);
            n.delete(id);
            return n;
        });
    }, [confirmRemoveId, loadInstalledModules, t]);

    const sortedModules = [...(installedModules || [])].sort((a, b) => byName(a.name, b.name));
    const installedIds = new Set(
        sortedModules.map((m) => {
            // m.name 은 "Minecraft" 같은 display name, path 에서 디렉토리 이름 추출
            const parts = (m.path || '').replace(/\\/g, '/').split('/');
            return parts[parts.length - 1]; // 마지막 경로 컴포넌트 = 모듈 디렉토리명
        }),
    );

    // 모듈 ID 추론 (디렉토리명 = path 마지막 컴포넌트)
    const getModuleId = (m: ModuleInfo) => {
        const parts = (m.path || '').replace(/\\/g, '/').split('/');
        return parts[parts.length - 1] || m.name?.toLowerCase();
    };

    // 매니페스트에서 미설치 모듈
    const uninstalledModules = manifestModules ? manifestModules.filter((m) => !installedIds.has(m.id)) : [];
    const runnableModules = uninstalledModules.filter((m) => m.platform_supported !== false);
    const unrunnableModules = uninstalledModules.filter((m) => m.platform_supported === false);

    return (
        <div className="saba-storage-tab-content">
            {versions.banner}
            {/* 설치된 모듈 */}
            <div data-spot="storage.installed" className="ss-store-header">
                <div className="ss-section-label" style={{ margin: 0 }}>
                    {t('saba_storage.installed_modules', '설치된 모듈')}
                </div>
                <button
                    className="ss-icon-btn"
                    disabled={refreshingModules}
                    onClick={handleRefreshModules}
                    data-tooltip={t('saba_storage.refresh_modules', '모듈 새로고침')}
                    aria-label={t('saba_storage.refresh_modules', '모듈 새로고침')}
                >
                    <Icon name={refreshingModules ? 'loader' : 'refresh'} size="sm" />
                </button>
            </div>
            {installedModules === null ? null : installedModules.length === 0 ? (
                // 받을 수 있는 목록이 바로 아래 보이므로 그림 대신 한 줄로 둔다
                <p className="ss-empty ss-installed-empty">{t('saba_storage.no_installed_modules', '설치된 모듈이 없습니다.')}</p>
            ) : (
                <div className="ss-cards">
                    {sortedModules.map((m, idx) => {
                        const moduleId = getModuleId(m);
                        const isRemoving = removingIds.has(moduleId);
                        const card: AddonCard = {
                            key: `module-${moduleId}`,
                            name: m.name,
                            kind: 'module',
                            version: m.version,
                            pinned: m.pinned,
                            pending_version: m.pending_version,
                        };
                        const progress = versions.progress(card);
                        return (
                            <div
                                className="ss-card"
                                key={m.name || idx}
                                onContextMenu={(e) =>
                                    openMenu(e, [
                                        ...versions.menuItems(card, isRemoving),
                                        'separator',
                                        {
                                            id: 'remove',
                                            label: t('saba_storage.remove', '제거'),
                                            icon: 'trash',
                                            danger: true,
                                            disabled: isRemoving,
                                            onSelect: () => setConfirmRemoveId(moduleId),
                                        },
                                    ])
                                }
                            >
                                <ModuleCardIcon icon={m.icon} />
                                <div className="ss-card-body">
                                    <span className="ss-card-name">
                                        {m.name}
                                        {m.version && <span className="ss-card-version">v{m.version}</span>}
                                        <TrustBadge trust={m.trust} />
                                        {versions.chip(card)}
                                    </span>
                                    {progress || (m.description && <span className="ss-card-desc">{m.description}</span>)}
                                    {versions.pinNote(card)}
                                </div>
                                <div className="ss-card-actions">
                                    <button
                                        className="ss-icon-btn danger"
                                        disabled={isRemoving}
                                        onClick={() => setConfirmRemoveId(moduleId)}
                                        data-tooltip={t('saba_storage.remove', '제거')}
                                        aria-label={t('saba_storage.remove', '제거')}
                                    >
                                        {isRemoving ? <SabaSpinner size="xs" /> : <Icon name="trash" size="sm" />}
                                    </button>
                                </div>
                            </div>
                        );
                    })}
                </div>
            )}

            {/* 받을 수 있는 모듈: 이 데몬에서 도는 것은 카드로, 돌지 않는 것은 접힌 묶음에 이유와 함께 */}
            <div className="ss-store-header">
                <div className="ss-section-label" style={{ margin: 0 }}>
                    {t('saba_storage.available_modules', '받을 수 있는 게임')}
                </div>
                <button
                    className="ss-icon-btn"
                    disabled={manifestLoading}
                    onClick={loadManifest}
                    data-tooltip={t('saba_storage.refresh_manifest', '목록 새로고침')}
                    aria-label={t('saba_storage.refresh_manifest', '목록 새로고침')}
                >
                    <Icon name={manifestLoading ? 'loader' : 'refresh'} size="sm" />
                </button>
            </div>
            {manifestLoading && <p className="ss-empty">{t('saba_storage.loading_manifest', '목록 받는 중…')}</p>}
            {manifestError && <SabaFail message={manifestError} />}
            {!manifestLoading && manifestModules !== null && uninstalledModules.length === 0 && !manifestError && (
                <p className="ss-empty">
                    {t('saba_storage.all_modules_installed', '사용 가능한 모든 모듈이 설치되어 있습니다.')}
                </p>
            )}
            {runnableModules.length > 0 && (
                <div data-spot="storage.catalog" className="ss-catalog">
                    {runnableModules.map((m) => {
                        const isInstalling = installingIds.has(m.id);
                        const result = installResults[m.id];
                        const name = localizedName(m, i18n.language) || m.id;
                        return (
                            <div className="ss-catalog-card" key={m.id}>
                                <div className="ss-catalog-top">
                                    <ModuleCardIcon icon={m.icon} />
                                    <div className="ss-catalog-title">
                                        <span className="ss-card-name">{name}</span>
                                        <span className="ss-card-version">
                                            {[m.version && `v${m.version}`, typeof m.size_bytes === 'number' && formatBytes(m.size_bytes)].filter(Boolean).join(' · ')}
                                        </span>
                                    </div>
                                </div>
                                <p className="ss-catalog-desc">{localizedDescription(m, i18n.language)}</p>
                                <div className="ss-catalog-foot">
                                    {result === 'ok' ? (
                                        <span className="ss-status-ok">
                                            <Icon name="checkCircle" size="sm" />
                                        </span>
                                    ) : result === 'error' ? (
                                        <span className="ss-status-err">
                                            <Icon name="xCircle" size="sm" />
                                        </span>
                                    ) : (
                                        <button
                                            className="btn btn-primary btn-sm ss-install-btn"
                                            type="button"
                                            disabled={isInstalling}
                                            onClick={() => handleInstallModule(m.id)}
                                        >
                                            {isInstalling ? <SabaSpinner size="xs" /> : null}
                                            {t('saba_storage.install_btn', '설치')}
                                        </button>
                                    )}
                                </div>
                            </div>
                        );
                    })}
                </div>
            )}
            {unrunnableModules.length > 0 && (
                <Disclosure
                    className="ss-unsupported"
                    summary={t('saba_storage.unsupported_here', {
                        count: unrunnableModules.length,
                        defaultValue: '이 기기에서는 실행할 수 없어요 · {{count}}',
                    })}
                >
                    {unrunnableModules.map((m) => {
                        const why = unsupportedText(t, m);
                        return (
                            <div className="ss-unsupported-row" key={m.id}>
                                <ModuleCardIcon icon={m.icon} />
                                <div className="ss-card-body">
                                    <span className="ss-card-name">{localizedName(m, i18n.language) || m.id}</span>
                                    <span className="ss-card-desc">{why?.detail}</span>
                                </div>
                                <span className="ss-unsupported-chip">{why?.label}</span>
                            </div>
                        );
                    })}
                </Disclosure>
            )}

            <ModuleMakerCard />

            {versions.dialogs}

            {confirmRemoveId && (
                <QuestionModal
                    title={t('saba_storage.remove_module_confirm_title', '모듈 제거')}
                    message={t('saba_storage.remove_confirm_msg', {
                        id: confirmRemoveId,
                        defaultValue: `'${confirmRemoveId}' 모듈을 제거하시겠습니까? 이 작업은 실행취소할 수 없으며, 해당 모듈을 사용하는 서버 인스턴스를 실행할 수 없게 됩니다.`,
                    })}
                    onConfirm={handleConfirmRemove}
                    onCancel={() => setConfirmRemoveId(null)}
                />
            )}
        </div>
    );
}

// ─────────────────────────────────────────────────────────────
// 익스텐션 탭
// ─────────────────────────────────────────────────────────────
type Described = Parameters<typeof localizedDescription>[0];

function ExtensionsTab() {
    const { t, i18n } = useTranslation('gui');
    const openMenu = useContextMenu();
    // 이 탭은 설정을 열 때 미리 띄워 두므로, 익스텐션 상태가 덜 채워져 있어도 빈 값으로 그린다
    const {
        extensions = [],
        refreshExtensions,
        manifestExtensions = [],
        availableUpdates = [],
        manifestLoading,
        installingIds = new Set<string>(),
        fetchManifest,
        installExtension,
        removeExtension,
        toggleExtension,
        loading,
    } = useExtensions();

    const [confirmRemoveId, setConfirmRemoveId] = useState<string | null>(null);
    const [togglingIds, setTogglingIds] = useState<Set<string>>(new Set());
    const [removingIds, setRemovingIds] = useState<Set<string>>(new Set());
    const [rescanning, setRescanning] = useState(false);
    const [iconUrls, setIconUrls] = useState<Record<string, string>>({}); // { extId: dataUrl }
    const [showStore, setShowStore] = useState(false);
    const versions = useAddonVersions(refreshExtensions);

    // 아이콘 로드: has_icon이 true인 익스텐션의 icon.png를 data URL로 가져옴
    useEffect(() => {
        const toLoad = extensions.filter((e) => e.has_icon && !iconUrls[e.id]);
        if (toLoad.length === 0) return;
        let cancelled = false;
        (async () => {
            const results: Record<string, string> = {};
            await Promise.all(
                toLoad.map(async (ext) => {
                    try {
                        const url = await window.api?.extensionIcon?.(ext.id);
                        if (url) results[ext.id] = url;
                    } catch (_) {}
                })
            );
            if (!cancelled && Object.keys(results).length > 0) {
                setIconUrls((prev) => ({ ...prev, ...results }));
            }
        })();
        return () => { cancelled = true; };
    }, [extensions]); // eslint-disable-line react-hooks/exhaustive-deps

    // 설치된 익스텐션 설명은 데몬이 GUI 언어로 골라 준다 → 언어가 바뀌면 다시 받는다
    const firstLanguage = useRef(i18n.language);
    useEffect(() => {
        if (firstLanguage.current === i18n.language) return;
        firstLanguage.current = i18n.language;
        refreshExtensions?.();
    }, [i18n.language]); // eslint-disable-line react-hooks/exhaustive-deps

    // 익스텐션 디렉토리 재스캔 + 목록 갱신
    const handleRescan = useCallback(async () => {
        setRescanning(true);
        try {
            await window.api?.extensionRescan?.();
            await refreshExtensions();
        } catch (_) {}
        setRescanning(false);
    }, [refreshExtensions]);

    // 스토어 열기 (모듈 탭 "더보기" 패턴과 동일)
    const handleShowStore = useCallback(async () => {
        if (manifestExtensions.length > 0) {
            setShowStore(true);
            return;
        }
        setShowStore(true);
        await fetchManifest();
    }, [manifestExtensions, fetchManifest]);

    const handleConfirmRemove = useCallback(async () => {
        const id = confirmRemoveId;
        setConfirmRemoveId(null);
        if (!id) return;
        setRemovingIds((prev) => new Set(prev).add(id));
        await removeExtension(id);
        setRemovingIds((prev) => {
            const n = new Set(prev);
            n.delete(id);
            return n;
        });
    }, [confirmRemoveId, removeExtension]);

    const handleToggle = useCallback(
        async (id: string, enable: boolean) => {
            setTogglingIds((prev) => new Set(prev).add(id));
            await toggleExtension(id, enable);
            setTogglingIds((prev) => {
                const n = new Set(prev);
                n.delete(id);
                return n;
            });
        },
        [toggleExtension],
    );

    const installedIds = new Set(extensions.map((e) => e.id));
    const uninstalled = manifestExtensions.filter((r) => !installedIds.has(r.id));

    return (
        <div className="saba-storage-tab-content">
            {versions.banner}
            {/* 설치됨 */}
            <div data-spot="extensions.installed" className="ss-store-header">
                <div className="ss-section-label" style={{ margin: 0 }}>
                    {t('extensions.installed_section', '설치됨')}
                </div>
                <button
                    className="ss-icon-btn"
                    disabled={rescanning}
                    onClick={handleRescan}
                    data-tooltip={t('extensions.rescan', '익스텐션 재스캔')}
                    aria-label={t('extensions.rescan', '익스텐션 재스캔')}
                >
                    <Icon name={rescanning ? 'loader' : 'refresh'} size="sm" />
                </button>
            </div>
            {extensions.length === 0 ? (
                // 목록을 받는 중에는 "없음"을 띄우지 않는다
                !loading && <SabaEmpty compact body={t('settings_modal.no_extensions', '설치된 익스텐션이 없습니다.')} />
            ) : (
                <div className="ss-cards">
                    {[...extensions].sort((a, b) => byName(a.name || a.id, b.name || b.id)).map((ext) => {
                        const updateInfo = availableUpdates.find((u) => u.id === ext.id);
                        const isRemoving = removingIds.has(ext.id);
                        const isBusy = isRemoving || installingIds.has(ext.id);
                        const card: AddonCard = {
                            key: `ext-${ext.id}`,
                            name: ext.name || ext.id,
                            kind: 'ext',
                            version: ext.version,
                            pinned: ext.pinned,
                            pending_version: ext.pending_version,
                        };
                        const progress = versions.progress(card);
                        return (
                            <div
                                className="ss-card"
                                key={ext.id}
                                onContextMenu={(e) =>
                                    openMenu(e, [
                                        updateInfo && {
                                            id: 'update',
                                            label: t('extensions.update_to', {
                                                version: updateInfo.latest_version,
                                                defaultValue: `v${updateInfo.latest_version}으로 업데이트`,
                                            }),
                                            icon: 'download',
                                            disabled: isBusy,
                                            onSelect: () =>
                                                installExtension(ext.id, { download_url: updateInfo.download_url }),
                                        },
                                        {
                                            id: 'toggle',
                                            label: ext.enabled ? t('context_menu.disable') : t('context_menu.enable'),
                                            icon: 'power',
                                            disabled: isBusy || togglingIds.has(ext.id),
                                            onSelect: () => handleToggle(ext.id, !ext.enabled),
                                        },
                                        ...versions.menuItems(card, isBusy),
                                        'separator',
                                        {
                                            id: 'remove',
                                            label: t('saba_storage.remove', '제거'),
                                            icon: 'trash',
                                            danger: true,
                                            disabled: isBusy,
                                            onSelect: () => setConfirmRemoveId(ext.id),
                                        },
                                    ])
                                }
                            >
                                <div className="ss-card-icon">
                                    {iconUrls[ext.id] ? (
                                        <img src={iconUrls[ext.id]} alt="" />
                                    ) : (
                                        <Icon name="extension" size="md" />
                                    )}
                                </div>
                                <div className="ss-card-body">
                                    <span className="ss-card-name">
                                        {ext.name}
                                        {ext.version && <span className="ss-card-version">v{ext.version}</span>}
                                        <TrustBadge trust={ext.trust} />
                                        {updateInfo && (
                                            <span className="ss-update-badge">v{updateInfo.latest_version}</span>
                                        )}
                                        {versions.chip(card)}
                                    </span>
                                    {progress ||
                                        ((localizedDescription(ext as Described, i18n.language) || ext.id) && (
                                            <span className="ss-card-desc">{localizedDescription(ext as Described, i18n.language) || ext.id}</span>
                                        ))}
                                    {versions.pinNote(card)}
                                    <ExtensionAuditNotice extId={ext.id} name={ext.name} />
                                    {ext.enabled && <ExtensionConfigForm ext={ext} />}
                                </div>
                                <div className="ss-card-actions">
                                    {updateInfo && (
                                        <button
                                            className="ss-icon-btn accent"
                                            disabled={isBusy}
                                            onClick={() =>
                                                installExtension(ext.id, { download_url: updateInfo.download_url })
                                            }
                                            data-tooltip={t('extensions.update_to', {
                                                version: updateInfo.latest_version,
                                                defaultValue: `v${updateInfo.latest_version}으로 업데이트`,
                                            })}
                                        >
                                            {installingIds.has(ext.id) ? (
                                                <SabaSpinner size="xs" />
                                            ) : (
                                                <Icon name="download" size="sm" />
                                            )}
                                        </button>
                                    )}
                                    <SabaToggle
                                        size="sm"
                                        checked={!!ext.enabled}
                                        disabled={isBusy || togglingIds.has(ext.id)}
                                        onChange={(checked) => handleToggle(ext.id, checked)}
                                        aria-label={ext.name || ext.id}
                                    />
                                    <button
                                        className="ss-icon-btn danger"
                                        disabled={isBusy}
                                        onClick={() => setConfirmRemoveId(ext.id)}
                                        data-tooltip={t('saba_storage.remove', '제거')}
                                        aria-label={t('saba_storage.remove', '제거')}
                                    >
                                        {isRemoving ? <SabaSpinner size="xs" /> : <Icon name="trash" size="sm" />}
                                    </button>
                                </div>
                            </div>
                        );
                    })}
                </div>
            )}

            {/* 스토어 */}
            <div data-spot="extensions.store" className="ss-store-header">
                <div className="ss-section-label" style={{ margin: 0 }}>
                    {t('extensions.store_section', '스토어')}
                </div>
                {showStore ? (
                    <button
                        className="ss-icon-btn"
                        disabled={manifestLoading}
                        onClick={fetchManifest}
                        data-tooltip={t('extensions.refresh_manifest', '매니페스트 새로고침')}
                        aria-label={t('extensions.refresh_manifest', '매니페스트 새로고침')}
                    >
                        <Icon name={manifestLoading ? 'loader' : 'refresh'} size="sm" />
                    </button>
                ) : (
                    <button
                        className="ss-icon-btn"
                        onClick={handleShowStore}
                        data-tooltip={t('saba_storage.show_more_extensions')}
                        aria-label={t('saba_storage.show_more_extensions')}
                    >
                        <Icon name="chevronDown" size="sm" />
                    </button>
                )}
            </div>

            {showStore && (
                <>
                    {manifestLoading ? (
                        <p className="ss-empty">{t('extensions.manifest_loading', '매니페스트를 가져오는 중...')}</p>
                    ) : uninstalled.length === 0 ? (
                        <p className="ss-empty">
                            {manifestExtensions.length === 0
                                ? t('extensions.store_empty', '새로고침 버튼으로 사용 가능한 익스텐션을 불러오세요.')
                                : t('extensions.all_installed', '모든 익스텐션이 설치되어 있습니다.')}
                        </p>
                    ) : (
                <div className="ss-cards">
                    {uninstalled.map((ext) => (
                        <div className="ss-card" key={ext.id}>
                            <div className="ss-card-icon">
                                <Icon name="extension" size="md" />
                            </div>
                            <div className="ss-card-body">
                                <span className="ss-card-name">
                                    {ext.name}
                                    {ext.version && <span className="ss-card-version">v{ext.version}</span>}
                                    {ext.author && <span className="ss-card-version">· {ext.author}</span>}
                                </span>
                                {(localizedDescription(ext as Described, i18n.language) || ext.id) && (
                                    <span className="ss-card-desc">{localizedDescription(ext as Described, i18n.language) || ext.id}</span>
                                )}
                            </div>
                            <div className="ss-card-actions">
                                <button
                                    className="ss-icon-btn accent"
                                    disabled={installingIds.has(ext.id)}
                                    onClick={() => installExtension(ext.id, { download_url: ext.download_url })}
                                    data-tooltip={t('extensions.install_btn', '설치')}
                                    aria-label={t('extensions.install_btn', '설치')}
                                >
                                    {installingIds.has(ext.id) ? (
                                        <SabaSpinner size="xs" />
                                    ) : (
                                        <Icon name="download" size="sm" />
                                    )}
                                </button>
                            </div>
                        </div>
                    ))}
                </div>
            )}
                </>
            )}

            {versions.dialogs}

            {confirmRemoveId && (
                <QuestionModal
                    title={t('saba_storage.remove_ext_confirm_title', '익스텐션 제거')}
                    message={t('saba_storage.remove_confirm_msg_ext', {
                        id: confirmRemoveId,
                        defaultValue: `'${confirmRemoveId}' 익스텐션을 제거하시겠습니까? 파일이 영구 삭제됩니다.`,
                    })}
                    onConfirm={handleConfirmRemove}
                    onCancel={() => setConfirmRemoveId(null)}
                />
            )}
        </div>
    );
}
export default SabaStorage;
