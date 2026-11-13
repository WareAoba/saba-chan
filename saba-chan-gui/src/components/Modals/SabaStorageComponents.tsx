import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { useAppVersion } from './AppVersion';
import { QuestionModal } from './Modals';
import { clearModuleAuditCache } from '../servers/ModuleAuditNotice';
import { byName } from '../../utils/helpers';
import type { ApiBody } from '../../types/daemon';
import { failed } from '../../utils/apiResult';
import { useUpdateStore, watchUpdates } from '../../stores/useUpdateStore';

/** 브리지는 던지지 않고 실패 본문으로 알린다 */
function throwIfRejected(res: ApiBody | null | undefined) {
    if (failed(res)) throw new Error(res?.error || 'failed');
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

export const parseComponents = (comps?: ApiBody[] | null) =>
    (comps || [])
        .filter((c) => {
            const k = typeof c.component === 'string' ? c.component : String(c.component);
            return k !== 'locales';
        })
        .map((c) => {
            const key = typeof c.component === 'string' ? c.component : String(c.component);
            // 종류는 데몬이 준다 (`kind`)
            const kind = c.kind;
            return {
                key,
                hidden: kind !== 'app',
                display: c.display_name || key,
                icon: kind === 'module' ? 'gamepad' : COMPONENT_ICONS[key] || 'server',
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

export function ComponentsTab({ devMode }: { devMode?: boolean }) {
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
    // 받는 진행과 상태는 데몬이 알린다 (업데이트 스토어). 이 화면은 묻지 않는다
    const downloadProgress = useUpdateStore((s) => s.progress);
    const status = useUpdateStore((s) => s.status);
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

    // 데몬이 알린 상태 (`update.status`, 읽은 것 포함)
    useEffect(() => {
        if (!status?.ok) return;
        setComponents(parseComponents(status.components));
        setStableWait(status.stable_wait || null);
    }, [status]);
    const refreshStatus = useCallback(async () => {
        await useUpdateStore.getState().refresh();
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
            try {
                const res = await window.api?.updaterDownload?.([key]);
                if (!res?.ok) setError(`${key}: ${res?.error || 'Download failed'}`);
                await refreshStatus();
            } catch (e) {
                setError(`${key}: ${(e as Error).message}`);
            }
            clearBusy(key);
        },
        [refreshStatus, markBusy, clearBusy],
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
    }, [components, refreshStatus, markBusy, clearBusy, t]);

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

        // 마운트 시: 상태를 보기 시작하고(이벤트 구독·한 번 읽기) 데몬 캐시가 비어 있으면 한 번만 check 실행
        const unwatch = watchUpdates();
        (async () => {
            try {
                // 읽은 상태는 위의 상태 효과가 화면에 싣는다
                const res = await useUpdateStore.getState().refresh();
                if (!res?.components?.length && mountedRef.current) {
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

        return () => {
            mountedRef.current = false;
            unwatch();
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
                        const pct = downloadProgress.percent ?? -1; // -1 = 전체 크기를 몰라 도는 링
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
