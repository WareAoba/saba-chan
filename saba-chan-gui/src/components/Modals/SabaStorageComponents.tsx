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
// 컴포넌트 탭: 새 버전이 있으면 "업데이트" 하나로 모든 구성요소(모듈·익스텐션 포함)를 올린다.
// 평소에는 데몬의 자동 업데이트가 하고, 이 버튼은 직접 올릴 때만 쓴다. 받기·적용 순서는 데몬이 정한다
// ─────────────────────────────────────────────────────────────

// locales는 백그라운드 자동 업데이트 — 다루지 않는다
const COMPONENT_ICONS: Record<string, string> = {
    gui: 'componentGui',
    'saba-core': 'componentCore',
    cli: 'componentCli',
    installer: 'componentInstaller',
};

// 목록 순서 (열 때마다 같게). 여기 없는 것은 뒤에 이름순
const COMPONENT_ORDER = ['saba-core', 'gui', 'cli', 'installer'];
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
            // 종류는 데몬이 준다 (`kind`). 모듈·익스텐션은 자기 탭에 있어 여기서는 세기만 한다
            const kind = c.kind;
            return {
                key,
                hidden: kind !== 'app',
                display: c.display_name || key,
                icon: COMPONENT_ICONS[key] || 'server',
                current_version: c.current_version || '—',
                latest_version: c.latest_version || null,
                update_available: !!c.update_available,
                // 적용하면 사바쨩이 다시 시작한다
                restarts: key === 'gui' || key === 'saba-core',
            };
        })
        .sort((a, b) => componentRank(a.key) - componentRank(b.key) || byName(a.key, b.key));

const MOCK_RELAY_URL = 'http://127.0.0.1:9876';

// 업데이트 방식 → 데몬 업데이트 설정 값
const UPDATE_MODES: Record<string, { auto_download: boolean; auto_apply: boolean }> = {
    check: { auto_download: false, auto_apply: false },
    download: { auto_download: true, auto_apply: false },
    apply: { auto_download: true, auto_apply: true },
};

export function ComponentsTab({ devMode }: { devMode?: boolean }) {
    const { t } = useTranslation('gui');

    const [components, setComponents] = useState<ReturnType<typeof parseComponents>>([]);
    const [checking, setChecking] = useState(false);
    const [updating, setUpdating] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [message, setMessage] = useState<string | null>(null);
    const [confirmRestart, setConfirmRestart] = useState(false);
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
        if (!status?.success) return;
        setComponents(parseComponents(status.components));
        setStableWait(status.stable_wait || null);
    }, [status]);

    // 수동 새로고침 — 데몬이 릴레이(/v1/updates/check)에 묻는다 (사용자 명시적 요청 시에만)
    const handleCheck = useCallback(async () => {
        setChecking(true);
        setError(null);
        setMessage(null);
        try {
            const res = await window.api?.updaterCheck?.();
            if (res?.success) {
                setComponents(parseComponents(res.components));
            } else {
                setError(res?.error || 'Unknown error');
            }
        } catch (e) {
            setError((e as Error).message);
        }
        setChecking(false);
    }, []);

    // 업데이트: 받을 것을 모두 받고 모두 적용한다(빈 목록 = 전부). 데몬·GUI가 바뀌면 데몬이 설치기를 띄운다
    const runUpdate = useCallback(async () => {
        const errors: string[] = [];
        const addError = (text: string) => {
            errors.push(text);
            setError(errors.join('\n'));
        };
        setUpdating(true);
        setError(null);
        setMessage(null);
        try {
            const downloaded = await window.api?.updaterDownload?.([]);
            if (failed(downloaded)) addError(downloaded?.error || t('saba_storage.download_failed'));
            const res = await window.api?.updaterApply?.([]);
            clearModuleAuditCache();
            // 일부만 적용돼도(success:false) 적용된 것은 알린다 (G-U9b)
            const applied: string[] = res?.applied || [];
            if (failed(res)) addError(res?.errors?.length > 0 ? res.errors.join('; ') : res?.error || t('saba_storage.apply_failed'));
            if (res?.requires_updater && res?.needs_updater?.length > 0) {
                const launchRes = await window.api?.updaterLaunchApply?.(res.needs_updater);
                if (launchRes?.ok === false) addError(launchRes?.error || t('saba_storage.updater_launch_failed'));
                // 설치기가 돌면 데몬이 내려간다. GUI가 남으면 다시 뜬 데몬에 붙는다
                else if (launchRes?.gui_restart === false)
                    setMessage(t('saba_storage.updater_background', { key: res.needs_updater.join(', ') }));
                return;
            }
            await useUpdateStore.getState().refresh();
            if (applied.length > 0) setMessage(t('saba_storage.apply_completed', { names: applied.join(', ') }));
            else if (errors.length === 0) setMessage(t('saba_storage.update_done'));
        } catch (e) {
            addError((e as Error).message);
        } finally {
            if (mountedRef.current) setUpdating(false);
        }
    }, [t]);

    const handleUpdate = useCallback(() => {
        if (components.some((c) => c.update_available && c.restarts)) setConfirmRestart(true);
        else runUpdate();
    }, [components, runUpdate]);

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
                    if (checkRes?.success && mountedRef.current) {
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

    const apps = components.filter((c) => !c.hidden);
    const updatable = components.filter((c) => c.update_available);
    const appUpdates = updatable.filter((c) => !c.hidden).length;
    const addonUpdates = updatable.length - appUpdates;
    const core = components.find((c) => c.key === 'saba-core');
    // 머리에 보일 새 버전: 코어의 새 버전, 코어가 그대로면 바뀌는 앱 구성요소의 것
    const newVersion = (core?.update_available && core.latest_version) || updatable.find((c) => !c.hidden)?.latest_version;
    const summary = [
        appUpdates > 0 && t('saba_storage.summary_apps', { count: appUpdates }),
        addonUpdates > 0 && t('saba_storage.summary_addons', { count: addonUpdates }),
    ].filter(Boolean);
    const progressName = downloadProgress.active
        ? components.find((c) => c.key === downloadProgress.component)?.display || downloadProgress.component
        : null;
    const pct = downloadProgress.percent ?? -1; // -1 = 전체 크기를 몰라 흐르는 막대
    const busy = checking || updating;

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
                                disabled={mockMode === null || busy}
                            />
                        </label>
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

                {components.length > 0 || checking ? (
                    <div className="ss-hero">
                        {updatable.length > 0 ? (
                            <>
                                <div className="ss-hero-top">
                                    <div className="ss-hero-text">
                                        <span className="ss-hero-title">
                                            {t('saba_storage.new_version')}
                                            {newVersion && <span className="ss-update-badge">v{newVersion}</span>}
                                        </span>
                                        <span className="ss-hero-sub">{t('saba_storage.update_summary', { items: summary.join(', ') })}</span>
                                    </div>
                                    <button
                                        type="button"
                                        className="btn btn-primary ss-update-btn"
                                        disabled={busy}
                                        onClick={handleUpdate}
                                    >
                                        {updating ? <SabaSpinner size="xs" /> : <Icon name="arrowUp" size="sm" />}
                                        {t(updating ? 'saba_storage.updating' : 'saba_storage.update')}
                                    </button>
                                </div>
                                {updating && progressName && (
                                    <div>
                                        <div className={clsx('ss-progress', { indeterminate: pct < 0 })}>
                                            <i style={pct < 0 ? undefined : { width: `${pct}%` }} />
                                        </div>
                                        <div className="ss-progress-row">
                                            <span>{t('saba_storage.downloading', { name: progressName })}</span>
                                            {pct >= 0 && <span>{pct}%</span>}
                                        </div>
                                    </div>
                                )}
                                {autoCheckEnabled && updateMode === 'apply' && (
                                    <span className="ss-hero-note">{t('saba_storage.auto_apply_note')}</span>
                                )}
                            </>
                        ) : (
                            <div className="ss-hero-top">
                                <div className="ss-hero-text">
                                    <span className="ss-hero-title">
                                        {checking ? t('saba_storage.checking') : t('saba_storage.latest_version')}
                                        {!checking && core && core.current_version !== '—' && (
                                            <span className="ss-ok-chip">
                                                <Icon name="check" size="xs" />v{core.current_version}
                                            </span>
                                        )}
                                    </span>
                                </div>
                                <button
                                    type="button"
                                    className="ss-icon-btn"
                                    disabled={busy}
                                    onClick={handleCheck}
                                    data-tooltip={t('saba_storage.refresh')}
                                    aria-label={t('saba_storage.refresh')}
                                >
                                    <Icon name={checking ? 'loader' : 'refresh'} size="sm" />
                                </button>
                            </div>
                        )}
                    </div>
                ) : null}

                <div className="ss-cards">
                    {apps.map((c) => (
                        <div key={c.key} className="ss-card">
                            <div className="ss-card-icon">
                                <Icon name={c.icon} size="md" />
                            </div>
                            <div className="ss-card-body">
                                <span className="ss-card-name">{c.display}</span>
                                <span className="ss-card-version">
                                    {appVersion.was(c.current_version)}v{c.current_version}
                                    {c.update_available && c.latest_version && (
                                        <>
                                            {' '}
                                            <span className="ss-version-arrow">→</span> v{c.latest_version}
                                        </>
                                    )}
                                </span>
                            </div>
                            {!c.update_available && (
                                <span className="ss-ok-chip">
                                    <Icon name="check" size="xs" />
                                    {t('saba_storage.up_to_date')}
                                </span>
                            )}
                        </div>
                    ))}
                </div>
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

            {appVersion.dialogs}
            {confirmRestart && (
                <QuestionModal
                    title={t('saba_storage.restart_required_title')}
                    message={t('saba_storage.restart_required_message')}
                    onConfirm={() => {
                        setConfirmRestart(false);
                        runUpdate();
                    }}
                    onCancel={() => setConfirmRestart(false)}
                />
            )}
        </>
    );
}
