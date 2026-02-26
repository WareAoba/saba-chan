import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useExtensions } from '../../contexts/ExtensionContext';
import { Icon } from '../Icon';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { QuestionModal } from './Modals';

// ─────────────────────────────────────────────────────────────
// 사바 스토리지 (구 업데이트 센터)
//
// 탭 3개:
//   [컴포넌트] — 기존 UpdatePanel 내용 그대로
//   [모듈]     — 설치된 모듈 + saba-chan-modules 레지스트리
//   [익스텐션] — 기존 익스텐션 탭 내용 이전
//
// @param {function} onBack  - 설정 일반 탭으로 돌아가기
// @param {boolean}  isExiting - 나가기 애니메이션 여부
// @param {boolean}  devMode   - 개발자 모드
// ─────────────────────────────────────────────────────────────
function SabaStorage({ onBack, isExiting, devMode }) {
    const { t } = useTranslation('gui');
    const [activeTab, setActiveTab] = useState('components');
    const tabsRef = useRef(null);
    const indicatorRef = useRef(null);
    const syncIndicator = useCallback(() => {
        const container = tabsRef.current;
        const indicator = indicatorRef.current;
        if (!container || !indicator) return;
        const activeBtn = container.querySelector('.saba-storage-tab.active');
        if (!activeBtn) return;
        indicator.style.left = `${activeBtn.offsetLeft}px`;
        indicator.style.width = `${activeBtn.offsetWidth}px`;
    }, []);
    // biome-ignore lint/correctness/useExhaustiveDependencies: activeTab triggers DOM re-render that syncIndicator reads via querySelector
    useEffect(() => {
        syncIndicator();
    }, [activeTab, syncIndicator]);

    return (
        <div className={clsx('update-panel', { exiting: isExiting })}>
            {/* 헤더 */}
            <div className="update-panel-header">
                <button className="update-panel-back" onClick={onBack} title="뒤로" disabled={isExiting}>
                    <Icon name="chevronLeft" size="sm" />
                </button>
                <h2 className="update-panel-title">{t('saba_storage.title', '사바 스토리지')}</h2>
            </div>

            {/* 내부 탭 */}
            <div className="saba-storage-tabs" ref={tabsRef}>
                <div className="saba-storage-tab-indicator" ref={indicatorRef} />
                {['components', 'modules', 'extensions'].map((tab) => (
                    <button
                        key={tab}
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

            <div className="saba-storage-content">
                {activeTab === 'components' && <ComponentsTab devMode={devMode} />}
                {activeTab === 'modules' && <ModulesTab />}
                {activeTab === 'extensions' && <ExtensionsTab />}
            </div>
        </div>
    );
}

// ─────────────────────────────────────────────────────────────
// 컴포넌트 탭 (기존 UpdatePanel 로직 그대로)
// ─────────────────────────────────────────────────────────────

// 컴포넌트 데이터 파싱 — 순수 함수이므로 컴포넌트 밖에 정의
const parseComponents = (comps) =>
    (comps || [])
        .filter((c) => {
            const k = typeof c.component === 'string' ? c.component : String(c.component);
            return !k.startsWith('module-');
        })
        .map((c) => {
            const key = typeof c.component === 'string' ? c.component : String(c.component);
            return {
                key,
                display: c.display_name || key,
                icon: key.startsWith('module-')
                    ? 'gamepad'
                    : key === 'gui'
                      ? 'monitor'
                      : key === 'cli'
                        ? 'terminal'
                        : key === 'discord_bot'
                          ? 'discord'
                          : 'server',
                current_version: c.current_version || '—',
                latest_version: c.latest_version || null,
                update_available: !!c.update_available,
                downloaded: !!c.downloaded,
                installed: !!c.installed,
                needsUpdater: key === 'gui' || key === 'saba-core',
            };
        });

function ComponentsTab({ devMode }) {
    const { t } = useTranslation('gui');

    const [components, setComponents] = useState([]);
    const [checking, setChecking] = useState(false);
    const [busyKeys, setBusyKeys] = useState(new Set());
    const [busyAll, setBusyAll] = useState(false);
    const [error, setError] = useState(null);
    const [message, setMessage] = useState(null);
    const [confirmRestart, setConfirmRestart] = useState(false);
    const [pendingRestartAction, setPendingRestartAction] = useState(null);
    const [mockMode, setMockMode] = useState(null);
    const [autoCheckEnabled, setAutoCheckEnabled] = useState(true);
    const pollRef = useRef(null);
    const mountedRef = useRef(true);

    const markBusy = useCallback((key) => setBusyKeys((prev) => new Set(prev).add(key)), []);
    const clearBusy = useCallback(
        (key) =>
            setBusyKeys((prev) => {
                const n = new Set(prev);
                n.delete(key);
                return n;
            }),
        [],
    );

    const handleToggleMock = useCallback(async (toMock) => {
        setError(null);
        setMessage(null);
        try {
            if (toMock) {
                await window.api?.updaterSetConfig?.({
                    api_base_url: 'http://127.0.0.1:9876',
                    github_owner: 'test-owner',
                    github_repo: 'saba-chan',
                });
                setMockMode(true);
                setMessage('Mock 주소로 전환 (localhost:9876)');
            } else {
                await window.api?.updaterSetConfig?.({
                    api_base_url: '',
                    github_owner: 'WareAoba',
                    github_repo: 'saba-chan',
                });
                setMockMode(false);
                setMessage('실제 GitHub API 주소로 전환');
            }
            setComponents([]);
        } catch (e) {
            setError(`주소 전환 실패: ${e.message}`);
        }
    }, []);

    const refreshStatus = useCallback(async () => {
        try {
            const res = await window.api?.updaterStatus?.();
            if (res?.ok && mountedRef.current) setComponents(parseComponents(res.components));
        } catch (_) {}
    }, []);

    // 수동 새로고침 — GitHub API 호출 (사용자 명시적 요청 시에만)
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
            setError(e.message);
        }
        setChecking(false);
    }, []);

    const handleDownloadOne = useCallback(
        async (key) => {
            markBusy(key);
            setError(null);
            try {
                const res = await window.api?.updaterDownload?.([key]);
                if (!res?.ok) setError(`${key}: ${res?.error || 'Download failed'}`);
                await refreshStatus();
            } catch (e) {
                setError(`${key}: ${e.message}`);
            }
            clearBusy(key);
        },
        [refreshStatus, markBusy, clearBusy],
    );

    const handleApplyOne = useCallback(
        async (key) => {
            if (key === 'gui') {
                setPendingRestartAction(() => async () => {
                    markBusy(key);
                    setError(null);
                    setMessage(null);
                    try {
                        const res = await window.api?.updaterApply?.([key]);
                        if (res?.requires_updater && res?.needs_updater?.length > 0) {
                            const launchRes = await window.api?.updaterLaunchApply?.(res.needs_updater);
                            if (launchRes?.ok === false)
                                setError(`${key}: ${launchRes?.error || '업데이터 실행 실패'}`);
                        } else if (res?.ok === false) {
                            setError(
                                `${key}: ${res?.errors?.length > 0 ? res.errors.join('; ') : res?.error || '적용 실패'}`,
                            );
                        }
                    } catch (e) {
                        setError(`${key}: ${e.message}`);
                    }
                    clearBusy(key);
                });
                setConfirmRestart(true);
                return;
            }
            if (key === 'discord_bot') {
                markBusy(key);
                setError(null);
                setMessage(null);
                try {
                    const botStatus = await window.api?.discordBotStatus?.();
                    const wasBotRunning = botStatus === 'running';
                    if (wasBotRunning) {
                        setMessage('Discord Bot 중지 중...');
                        await window.api?.discordBotStop?.();
                        await new Promise((r) => setTimeout(r, 2000));
                    }
                    setMessage('Discord Bot 파일 교체 중...');
                    const res = await window.api?.updaterApply?.([key]);
                    if (res?.ok === false) {
                        setError(
                            `${key}: ${res?.errors?.length > 0 ? res.errors.join('; ') : res?.error || '적용 실패'}`,
                        );
                    } else if (res?.applied?.length > 0) {
                        setMessage(`${res.applied.join(', ')} 적용 완료`);
                        if (wasBotRunning) {
                            setMessage('Discord Bot 재시작 중...');
                            const settings = await window.api?.settingsLoad?.();
                            const botConfig = await window.api?.botConfigLoad?.();
                            const token = settings?.discordToken;
                            if (token) {
                                const startRes = await window.api?.discordBotStart?.({
                                    token,
                                    prefix: botConfig?.prefix || '!saba',
                                    moduleAliases: botConfig?.moduleAliases || {},
                                    commandAliases: botConfig?.commandAliases || {},
                                });
                                if (startRes?.error) setError(`Discord Bot 재시작 실패: ${startRes.error}`);
                                else setMessage('Discord Bot 업데이트 후 재시작 완료');
                            } else {
                                setMessage('Discord Bot 업데이트 완료 (토큰 없음 — 수동 재시작 필요)');
                            }
                        }
                    }
                    await refreshStatus();
                } catch (e) {
                    setError(`${key}: ${e.message}`);
                }
                clearBusy(key);
                return;
            }
            markBusy(key);
            setError(null);
            setMessage(null);
            try {
                const res = await window.api?.updaterApply?.([key]);
                if (res?.ok === false) {
                    setError(`${key}: ${res?.errors?.length > 0 ? res.errors.join('; ') : res?.error || '적용 실패'}`);
                } else {
                    if (res?.requires_updater && res?.needs_updater?.length > 0) {
                        const launchRes = await window.api?.updaterLaunchApply?.(res.needs_updater);
                        if (launchRes?.ok === false) setError(`${key}: ${launchRes?.error || '업데이터 실행 실패'}`);
                        else setMessage(`${key}: 업데이터가 백그라운드에서 파일을 교체합니다`);
                    } else if (res?.applied?.length > 0) {
                        setMessage(`${res.applied.join(', ')} 적용 완료`);
                    } else {
                        setMessage(`${key} 적용 요청 완료`);
                    }
                }
                await refreshStatus();
            } catch (e) {
                setError(`${key}: ${e.message}`);
            }
            clearBusy(key);
        },
        [refreshStatus, markBusy, clearBusy],
    );

    const executeUpdateAll = useCallback(async () => {
        const updatable = components.filter((c) => c.update_available);
        if (updatable.length === 0) return;
        let hadError = false;
        setBusyAll(true);
        setError(null);
        setMessage(null);
        const allKeys = updatable.map((c) => c.key);
        const updaterTargets = ['gui', 'saba-core'];
        const daemonKeys = allKeys.filter((k) => !updaterTargets.includes(k));
        const updaterKeys = allKeys.filter((k) => updaterTargets.includes(k));
        const hasDiscordBot = allKeys.includes('discord_bot');
        let wasBotRunning = false;
        if (hasDiscordBot) {
            try {
                const s = await window.api?.discordBotStatus?.();
                wasBotRunning = s === 'running';
                if (wasBotRunning) {
                    await window.api?.discordBotStop?.();
                    await new Promise((r) => setTimeout(r, 2000));
                }
            } catch (_) {}
        }
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
                setError((prev) => (prev ? prev + '\n' : '') + `${key}: ${e.message}`);
            }
            clearBusy(key);
        }
        await refreshStatus();
        let daemonApplied = [];
        if (daemonKeys.length > 0) {
            for (const key of daemonKeys) markBusy(key);
            try {
                const res = await window.api?.updaterApply?.(daemonKeys);
                if (res?.ok === false) {
                    hadError = true;
                    setError(
                        (prev) =>
                            (prev ? prev + '\n' : '') +
                            (res?.errors?.length > 0 ? res.errors.join('; ') : res?.error || '적용 실패'),
                    );
                } else {
                    daemonApplied = res?.applied || [];
                }
            } catch (e) {
                hadError = true;
                setError((prev) => (prev ? prev + '\n' : '') + e.message);
            }
            for (const key of daemonKeys) clearBusy(key);
        }
        if (updaterKeys.length > 0) {
            try {
                const launchRes = await window.api?.updaterLaunchApply?.(updaterKeys);
                if (launchRes?.ok === false) {
                    hadError = true;
                    setError((prev) => (prev ? prev + '\n' : '') + (launchRes?.error || '업데이터 실행 실패'));
                }
            } catch (e) {
                hadError = true;
                setError((prev) => (prev ? prev + '\n' : '') + `업데이터: ${e.message}`);
            }
        }
        await refreshStatus();
        if (hasDiscordBot && wasBotRunning) {
            try {
                const settings = await window.api?.settingsLoad?.();
                const botConfig = await window.api?.botConfigLoad?.();
                const token = settings?.discordToken;
                if (token) {
                    const s = await window.api?.discordBotStart?.({
                        token,
                        prefix: botConfig?.prefix || '!saba',
                        moduleAliases: botConfig?.moduleAliases || {},
                        commandAliases: botConfig?.commandAliases || {},
                    });
                    if (s?.error) {
                        hadError = true;
                        setError((prev) => (prev ? prev + '\n' : '') + `Discord Bot 재시작 실패: ${s.error}`);
                    }
                }
            } catch (e) {
                hadError = true;
                setError((prev) => (prev ? prev + '\n' : '') + `Discord Bot 재시작: ${e.message}`);
            }
        }
        const summary = [];
        if (daemonApplied.length > 0) summary.push(`${daemonApplied.join(', ')} 적용 완료`);
        if (updaterKeys.length > 0) summary.push('업데이터로 적용 진행 중');
        if (summary.length > 0) setMessage(summary.join(' · '));
        else if (!hadError) setMessage('업데이트 처리 완료');
        setBusyAll(false);
    }, [components, refreshStatus, markBusy, clearBusy]);

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

    const handleAutoCheckToggle = useCallback(async (enabled) => {
        setAutoCheckEnabled(enabled);
        try {
            await window.api?.updaterSetConfig?.({ enabled });
        } catch (e) {
            console.error('Failed to set auto-check config:', e);
            setAutoCheckEnabled(!enabled);
        }
    }, []);

    useEffect(() => {
        mountedRef.current = true;
        (async () => {
            try {
                const cfgRes = await window.api?.updaterGetConfig?.();
                const cfg = cfgRes?.config || cfgRes;
                const isMock = !!(cfg?.api_base_url && cfg.api_base_url.includes('127.0.0.1'));
                if (mountedRef.current) {
                    setMockMode(isMock);
                    setAutoCheckEnabled(cfg?.enabled !== false);
                }
            } catch (_) {
                if (mountedRef.current) setMockMode(false);
            }
        })();

        // 마운트 시: 데몬 캐시 읽기 → 비어있으면 한 번만 check 실행
        (async () => {
            try {
                const res = await window.api?.updaterStatus?.();
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
                if (res?.ok && mountedRef.current) setComponents(parseComponents(res.components));
            } catch (_) {}
        }, 30000);

        return () => {
            mountedRef.current = false;
            clearInterval(pollRef.current);
        };
        // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only initialization — async tasks use refs for mount guard
    }, []);

    const updatable = components.filter((c) => c.update_available);
    const allUpToDate = components.length > 0 && updatable.length === 0;
    const anyBusy = checking || busyAll || busyKeys.size > 0;

    return (
        <>
            <div className="saba-storage-tab-content">
                {devMode && (
                    <div className="update-panel-header" style={{ paddingTop: 0, paddingBottom: '8px' }}>
                        <label
                            className="update-mock-toggle"
                            title={mockMode ? 'Mock 서버 (테스트)' : '실제 GitHub API'}
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
                        <Icon name="loader" size="sm" /> 업데이트 확인 중...
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
                                title={t('saba_storage.update_all', '모두 업데이트')}
                            >
                                <Icon name="download" size="sm" />
                            </button>
                        )}
                        <button
                            className="ss-icon-btn"
                            disabled={anyBusy}
                            onClick={handleCheck}
                            title={t('saba_storage.refresh', '새로고침')}
                        >
                            <Icon name={checking ? 'loader' : 'refresh'} size="sm" />
                        </button>
                    </div>
                </div>

                <div className="ss-cards">
                    {components.map((c) => {
                        const isBusy = busyKeys.has(c.key) || busyAll;
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
                                    <span className="ss-card-version">v{c.current_version}</span>
                                </div>
                                <div className="ss-card-actions">
                                    {c.update_available && !c.downloaded && (
                                        <button
                                            className="ss-icon-btn accent"
                                            disabled={isBusy}
                                            onClick={() => handleDownloadOne(c.key)}
                                            title="다운로드"
                                        >
                                            {isBusy ? <SabaSpinner size="xs" /> : <Icon name="download" size="sm" />}
                                        </button>
                                    )}
                                    {c.update_available && c.downloaded && (
                                        <button
                                            className={clsx('ss-icon-btn', c.needsUpdater ? 'warning' : 'accent')}
                                            disabled={isBusy}
                                            onClick={() => handleApplyOne(c.key)}
                                            title={c.needsUpdater ? '적용 (재시작 필요)' : '적용'}
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
                        {t('updates.auto_check', '자동으로 업데이트 확인')}
                    </span>
                    <SabaToggle checked={autoCheckEnabled} onChange={(checked) => handleAutoCheckToggle(checked)} />
                </label>
            </div>

            <div className="update-panel-actions" />

            {confirmRestart && (
                <QuestionModal
                    title="재시작 필요"
                    message="업데이트를 적용하면 프로그램이 재시작됩니다. 계속하시겠습니까?"
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
function ModuleCardIcon({ icon }) {
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
    const { t } = useTranslation('gui');
    const [installedModules, setInstalledModules] = useState([]);
    const [registryModules, setRegistryModules] = useState(null); // null = 미로드, [] = 로드됨
    const [registryLoading, setRegistryLoading] = useState(false);
    const [registryError, setRegistryError] = useState(null);
    const [installingIds, setInstallingIds] = useState(new Set());
    const [installResults, setInstallResults] = useState({}); // { id: 'ok'|'error' }
    const [showRegistry, setShowRegistry] = useState(false);
    const [removingIds, setRemovingIds] = useState(new Set());
    const [confirmRemoveId, setConfirmRemoveId] = useState(null);
    const [refreshingModules, setRefreshingModules] = useState(false);

    // 설치된 모듈 로드
    const loadInstalledModules = useCallback(async () => {
        try {
            const res = await window.api?.moduleList?.();
            if (res?.modules) setInstalledModules(res.modules);
        } catch (_) {}
    }, []);

    // 모듈 캐시 새로고침 (디스크 재스캔)
    const handleRefreshModules = useCallback(async () => {
        setRefreshingModules(true);
        try {
            const res = await window.api?.moduleRefresh?.();
            if (res?.modules) setInstalledModules(res.modules);
        } catch (_) {}
        setRefreshingModules(false);
    }, []);

    useEffect(() => {
        loadInstalledModules();
    }, [loadInstalledModules]);

    const handleShowRegistry = useCallback(async () => {
        if (registryModules !== null) {
            setShowRegistry(true);
            return;
        }
        setShowRegistry(true);
        setRegistryLoading(true);
        setRegistryError(null);
        try {
            const res = await window.api?.moduleRegistry?.();
            if (res?.ok && res.registry?.modules) {
                // registry.modules: { id: { version, display_name, description, ... } }
                const mods = Object.entries(res.registry.modules).map(([id, info]) => ({
                    id,
                    ...info,
                }));
                setRegistryModules(mods);
            } else {
                setRegistryError(
                    res?.error || t('saba_storage.registry_fetch_failed', '레지스트리를 가져오지 못했습니다.'),
                );
                setRegistryModules([]);
            }
        } catch (e) {
            setRegistryError(e.message);
            setRegistryModules([]);
        }
        setRegistryLoading(false);
    }, [registryModules, t]);

    const handleRefreshRegistry = useCallback(async () => {
        setRegistryModules(null);
        setRegistryLoading(true);
        setRegistryError(null);
        try {
            const res = await window.api?.moduleRegistry?.();
            if (res?.ok && res.registry?.modules) {
                const mods = Object.entries(res.registry.modules).map(([id, info]) => ({ id, ...info }));
                setRegistryModules(mods);
            } else {
                setRegistryError(
                    res?.error || t('saba_storage.registry_fetch_failed', '레지스트리를 가져오지 못했습니다.'),
                );
                setRegistryModules([]);
            }
        } catch (e) {
            setRegistryError(e.message);
            setRegistryModules([]);
        }
        setRegistryLoading(false);
    }, [t]);

    const handleInstallModule = useCallback(async (moduleId) => {
        setInstallingIds((prev) => new Set(prev).add(moduleId));
        setInstallResults((prev) => ({ ...prev, [moduleId]: null }));
        try {
            const res = await window.api?.moduleInstallFromRegistry?.(moduleId);
            if (res?.ok) {
                setInstallResults((prev) => ({ ...prev, [moduleId]: 'ok' }));
                // 설치 후 로컬 모듈 목록 갱신
                const listRes = await window.api?.moduleRefresh?.();
                if (listRes?.modules) setInstalledModules(listRes.modules);
            } else {
                setInstallResults((prev) => ({ ...prev, [moduleId]: 'error' }));
            }
        } catch (_) {
            setInstallResults((prev) => ({ ...prev, [moduleId]: 'error' }));
        }
        setInstallingIds((prev) => {
            const n = new Set(prev);
            n.delete(moduleId);
            return n;
        });
    }, []);

    const handleConfirmRemove = useCallback(async () => {
        const id = confirmRemoveId;
        setConfirmRemoveId(null);
        if (!id) return;
        setRemovingIds((prev) => new Set(prev).add(id));
        try {
            const res = await window.api?.moduleRemove?.(id);
            if (res?.ok) await loadInstalledModules();
        } catch (_) {}
        setRemovingIds((prev) => {
            const n = new Set(prev);
            n.delete(id);
            return n;
        });
    }, [confirmRemoveId, loadInstalledModules]);

    const installedIds = new Set(
        installedModules.map((m) => {
            // m.name 은 "Minecraft" 같은 display name, path 에서 디렉토리 이름 추출
            const parts = (m.path || '').replace(/\\/g, '/').split('/');
            return parts[parts.length - 1]; // 마지막 경로 컴포넌트 = 모듈 디렉토리명
        }),
    );

    // 모듈 ID 추론 (디렉토리명 = path 마지막 컴포넌트)
    const getModuleId = (m) => {
        const parts = (m.path || '').replace(/\\/g, '/').split('/');
        return parts[parts.length - 1] || m.name?.toLowerCase();
    };

    // 레지스트리에서 미설치 모듈
    const uninstalledModules = registryModules ? registryModules.filter((m) => !installedIds.has(m.id)) : [];

    return (
        <div className="saba-storage-tab-content">
            {/* 설치된 모듈 */}
            <div className="ss-store-header">
                <div className="ss-section-label" style={{ margin: 0 }}>
                    {t('saba_storage.installed_modules', '설치된 모듈')}
                </div>
                <button
                    className="ss-icon-btn"
                    disabled={refreshingModules}
                    onClick={handleRefreshModules}
                    title={t('saba_storage.refresh_modules', '모듈 새로고침')}
                >
                    <Icon name={refreshingModules ? 'loader' : 'refresh'} size="sm" />
                </button>
            </div>
            {installedModules.length === 0 ? (
                <p className="ss-empty">{t('saba_storage.no_installed_modules', '설치된 모듈이 없습니다.')}</p>
            ) : (
                <div className="ss-cards">
                    {installedModules.map((m, idx) => {
                        const moduleId = getModuleId(m);
                        const isRemoving = removingIds.has(moduleId);
                        return (
                            <div className="ss-card" key={m.name || idx}>
                                <ModuleCardIcon icon={m.icon} />
                                <div className="ss-card-body">
                                    <span className="ss-card-name">
                                        {m.name}
                                        {m.version && <span className="ss-card-version">v{m.version}</span>}
                                    </span>
                                    {m.description && <span className="ss-card-desc">{m.description}</span>}
                                </div>
                                <div className="ss-card-actions">
                                    <button
                                        className="ss-icon-btn danger"
                                        disabled={isRemoving}
                                        onClick={() => setConfirmRemoveId(moduleId)}
                                        title={t('saba_storage.remove', '제거')}
                                    >
                                        {isRemoving ? <SabaSpinner size="xs" /> : <Icon name="trash" size="sm" />}
                                    </button>
                                </div>
                            </div>
                        );
                    })}
                </div>
            )}

            {/* 스토어 헤더 */}
            <div className="ss-store-header">
                <div className="ss-section-label" style={{ margin: 0 }}>
                    {t('saba_storage.available_modules', '사용 가능한 모듈')}
                </div>
                {showRegistry ? (
                    <button
                        className="ss-icon-btn"
                        disabled={registryLoading}
                        onClick={handleRefreshRegistry}
                        title={t('saba_storage.refresh_registry', '레지스트리 새로고침')}
                    >
                        <Icon name={registryLoading ? 'loader' : 'refresh'} size="sm" />
                    </button>
                ) : (
                    <button
                        className="ss-icon-btn"
                        onClick={handleShowRegistry}
                        title={t('saba_storage.show_more_modules', '더 많은 모듈 표시')}
                    >
                        <Icon name="chevronDown" size="sm" />
                    </button>
                )}
            </div>

            {showRegistry && (
                <>
                    {registryLoading && (
                        <p className="ss-empty">{t('saba_storage.loading_registry', '레지스트리 로드 중...')}</p>
                    )}
                    {registryError && (
                        <div className="update-modal-status-bar error">
                            <Icon name="xCircle" size="sm" /> {registryError}
                        </div>
                    )}
                    {!registryLoading &&
                        registryModules !== null &&
                        uninstalledModules.length === 0 &&
                        !registryError && (
                            <p className="ss-empty">
                                {t('saba_storage.all_modules_installed', '사용 가능한 모든 모듈이 설치되어 있습니다.')}
                            </p>
                        )}
                    <div className="ss-cards">
                        {uninstalledModules.map((m) => {
                            const isInstalling = installingIds.has(m.id);
                            const result = installResults[m.id];
                            return (
                                <div className="ss-card" key={m.id}>
                                    <div className="ss-card-body">
                                        <span className="ss-card-name">
                                            {m.display_name || m.id}
                                            {m.version && <span className="ss-card-version">v{m.version}</span>}
                                        </span>
                                        {m.description && <span className="ss-card-desc">{m.description}</span>}
                                    </div>
                                    <div className="ss-card-actions">
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
                                                className="ss-icon-btn accent"
                                                disabled={isInstalling}
                                                onClick={() => handleInstallModule(m.id)}
                                                title={t('saba_storage.install_btn', '설치')}
                                            >
                                                {isInstalling ? (
                                                    <SabaSpinner size="xs" />
                                                ) : (
                                                    <Icon name="download" size="sm" />
                                                )}
                                            </button>
                                        )}
                                    </div>
                                </div>
                            );
                        })}
                    </div>
                </>
            )}

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
function ExtensionsTab() {
    const { t } = useTranslation('gui');
    const {
        extensions,
        refreshExtensions,
        registryExtensions,
        availableUpdates,
        registryLoading,
        installingIds,
        fetchRegistry,
        installExtension,
        removeExtension,
    } = useExtensions();

    const [confirmRemoveId, setConfirmRemoveId] = useState(null);
    const [removingIds, setRemovingIds] = useState(new Set());
    const [rescanning, setRescanning] = useState(false);

    // 익스텐션 디렉토리 재스캔 + 목록 갱신
    const handleRescan = useCallback(async () => {
        setRescanning(true);
        try {
            await window.api?.extensionRescan?.();
            await refreshExtensions();
        } catch (_) {}
        setRescanning(false);
    }, [refreshExtensions]);

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

    const installedIds = new Set(extensions.map((e) => e.id));
    const uninstalled = registryExtensions.filter((r) => !installedIds.has(r.id));

    return (
        <div className="saba-storage-tab-content">
            {/* 설치됨 */}
            <div className="ss-store-header">
                <div className="ss-section-label" style={{ margin: 0 }}>
                    {t('extensions.installed_section', '설치됨')}
                </div>
                <button
                    className="ss-icon-btn"
                    disabled={rescanning}
                    onClick={handleRescan}
                    title={t('extensions.rescan', '익스텐션 재스캔')}
                >
                    <Icon name={rescanning ? 'loader' : 'refresh'} size="sm" />
                </button>
            </div>
            {extensions.length === 0 ? (
                <p className="ss-empty">{t('settings_modal.no_extensions', '설치된 익스텐션이 없습니다.')}</p>
            ) : (
                <div className="ss-cards">
                    {extensions.map((ext) => {
                        const updateInfo = availableUpdates.find((u) => u.id === ext.id);
                        const isRemoving = removingIds.has(ext.id);
                        const isBusy = isRemoving || installingIds.has(ext.id);
                        return (
                            <div className="ss-card" key={ext.id}>
                                <div className="ss-card-icon">
                                    <Icon name="extension" size="md" />
                                </div>
                                <div className="ss-card-body">
                                    <span className="ss-card-name">
                                        {ext.name}
                                        {ext.version && <span className="ss-card-version">v{ext.version}</span>}
                                        {updateInfo && (
                                            <span className="ss-update-badge">v{updateInfo.latest_version}</span>
                                        )}
                                    </span>
                                    {(ext.description || ext.id) && (
                                        <span className="ss-card-desc">{ext.description || ext.id}</span>
                                    )}
                                </div>
                                <div className="ss-card-actions">
                                    {updateInfo && (
                                        <button
                                            className="ss-icon-btn accent"
                                            disabled={isBusy}
                                            onClick={() =>
                                                installExtension(ext.id, { download_url: updateInfo.download_url })
                                            }
                                            title={t('extensions.update_to', {
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
                                    <button
                                        className="ss-icon-btn danger"
                                        disabled={isBusy}
                                        onClick={() => setConfirmRemoveId(ext.id)}
                                        title={t('saba_storage.remove', '제거')}
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
            <div className="ss-store-header">
                <div className="ss-section-label" style={{ margin: 0 }}>
                    {t('extensions.store_section', '스토어')}
                </div>
                <button
                    className="ss-icon-btn"
                    disabled={registryLoading}
                    onClick={fetchRegistry}
                    title={t('extensions.refresh_registry', '레지스트리 새로고침')}
                >
                    <Icon name={registryLoading ? 'loader' : 'refresh'} size="sm" />
                </button>
            </div>
            {registryLoading ? (
                <p className="ss-empty">{t('extensions.registry_loading', '레지스트리를 가져오는 중...')}</p>
            ) : uninstalled.length === 0 ? (
                <p className="ss-empty">
                    {registryExtensions.length === 0
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
                                {(ext.description || ext.id) && (
                                    <span className="ss-card-desc">{ext.description || ext.id}</span>
                                )}
                            </div>
                            <div className="ss-card-actions">
                                <button
                                    className="ss-icon-btn accent"
                                    disabled={installingIds.has(ext.id)}
                                    onClick={() => installExtension(ext.id, { download_url: ext.download_url })}
                                    title={t('extensions.install_btn', '설치')}
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
