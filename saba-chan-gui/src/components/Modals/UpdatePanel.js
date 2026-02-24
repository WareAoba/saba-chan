import React, { useState, useCallback, useRef, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { SabaToggle, SabaSpinner } from '../ui/SabaUI';
import { QuestionModal } from './Modals';

/**
 * 인앱 업데이트 패널 (설정 모달 내부에 마운트)
 *
 * 데몬 HTTP API를 통해 업데이트를 확인/다운로드/적용합니다.
 * - 모듈: 데몬이 직접 적용 (프로세스 중단 불필요)
 * - 데몬/GUI/CLI: 업데이터 exe를 스폰하여 파일 교체
 *
 * 각 컴포넌트마다 독립적인 다운로드/적용 버튼이 있으며,
 * 하단 "모두 업데이트" 버튼으로 전체 일괄 처리 가능합니다.
 *
 * @param {function} onBack - 설정 일반 탭으로 돌아가기
 */
function UpdatePanel({ onBack, isExiting, devMode }) {
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

    // ── 헬퍼: 컴포넌트 데이터 파싱 ──
    const parseComponents = (comps) => (comps || []).map(c => {
        const key = typeof c.component === 'string' ? c.component : String(c.component);
        return {
            key,
            display: c.display_name || key,
            icon: key.startsWith('module-') ? 'gamepad'
                : key === 'gui' ? 'monitor'
                : key === 'cli' ? 'terminal'
                : key === 'discord_bot' ? 'discord'
                : 'server',
            current_version: c.current_version || '—',
            latest_version: c.latest_version || null,
            update_available: !!c.update_available,
            downloaded: !!c.downloaded,
            installed: !!c.installed,
            needsUpdater: key === 'gui' || key === 'saba-core',
        };
    });

    // ── busy 키 관리 ──
    const markBusy = (key) => setBusyKeys(prev => new Set(prev).add(key));
    const clearBusy = (key) => setBusyKeys(prev => { const n = new Set(prev); n.delete(key); return n; });

    // ── Mock/Real 페치 주소 토글 ──
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

    // ── 상태 새로고침 ──
    const refreshStatus = useCallback(async () => {
        try {
            const res = await window.api?.updaterStatus?.();
            if (res?.ok && mountedRef.current) setComponents(parseComponents(res.components));
        } catch (_) {}
    }, []);

    // ── 업데이트 확인 ──
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

    // ── 개별 컴포넌트 다운로드 ──
    const handleDownloadOne = useCallback(async (key) => {
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
    }, [refreshStatus]);

    // ── 개별 컴포넌트 적용 ──
    const handleApplyOne = useCallback(async (key) => {
        if (key === 'gui') {
            setPendingRestartAction(() => async () => {
                markBusy(key);
                setError(null);
                setMessage(null);
                try {
                    const res = await window.api?.updaterApply?.([key]);
                    if (res?.requires_updater && res?.needs_updater?.length > 0) {
                        const launchRes = await window.api?.updaterLaunchApply?.(res.needs_updater);
                        if (launchRes?.ok === false) {
                            setError(`${key}: ${launchRes?.error || '업데이터 실행 실패'}`);
                        }
                    } else if (res?.ok === false) {
                        const errDetail = res?.errors?.length > 0
                            ? res.errors.join('; ') : (res?.error || '적용 실패');
                        setError(`${key}: ${errDetail}`);
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
                    await new Promise(r => setTimeout(r, 2000));
                }
                setMessage('Discord Bot 파일 교체 중...');
                const res = await window.api?.updaterApply?.([key]);
                if (res?.ok === false) {
                    const errDetail = res?.errors?.length > 0
                        ? res.errors.join('; ') : (res?.error || '적용 실패');
                    setError(`${key}: ${errDetail}`);
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
                            if (startRes?.error) {
                                setError(`Discord Bot 재시작 실패: ${startRes.error}`);
                            } else {
                                setMessage('Discord Bot 업데이트 후 재시작 완료');
                            }
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
                const errDetail = res?.errors?.length > 0
                    ? res.errors.join('; ')
                    : (res?.error || '적용 실패');
                setError(`${key}: ${errDetail}`);
            } else {
                if (res?.requires_updater && res?.needs_updater?.length > 0) {
                    const launchRes = await window.api?.updaterLaunchApply?.(res.needs_updater);
                    if (launchRes?.ok === false) {
                        setError(`${key}: ${launchRes?.error || '업데이터 실행 실패'}`);
                    } else {
                        setMessage(`${key}: 업데이터가 백그라운드에서 파일을 교체합니다`);
                    }
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
    }, [refreshStatus]);

    // ── 모두 업데이트 내부 실행 ──
    const executeUpdateAll = useCallback(async () => {
        const updatable = components.filter(c => c.update_available);
        if (updatable.length === 0) return;

        setBusyAll(true);
        setError(null);
        setMessage(null);

        const allKeys = updatable.map(c => c.key);
        const updaterTargets = ['gui', 'saba-core'];
        const daemonKeys = allKeys.filter(k => !updaterTargets.includes(k));
        const updaterKeys = allKeys.filter(k => updaterTargets.includes(k));
        const hasDiscordBot = allKeys.includes('discord_bot');

        let wasBotRunning = false;
        if (hasDiscordBot) {
            try {
                const botStatus = await window.api?.discordBotStatus?.();
                wasBotRunning = botStatus === 'running';
                if (wasBotRunning) {
                    await window.api?.discordBotStop?.();
                    await new Promise(r => setTimeout(r, 2000));
                }
            } catch (e) { /* ignore */ }
        }

        for (const key of allKeys) markBusy(key);
        for (const key of allKeys) {
            try {
                const res = await window.api?.updaterDownload?.([key]);
                if (!res?.ok) setError(prev => (prev ? prev + '\n' : '') + `${key}: ${res?.error}`);
            } catch (e) {
                setError(prev => (prev ? prev + '\n' : '') + `${key}: ${e.message}`);
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
                    const errDetail = res?.errors?.length > 0
                        ? res.errors.join('; ')
                        : (res?.error || '적용 실패');
                    setError(prev => (prev ? prev + '\n' : '') + errDetail);
                } else {
                    daemonApplied = res?.applied || [];
                }
            } catch (e) {
                setError(prev => (prev ? prev + '\n' : '') + e.message);
            }
            for (const key of daemonKeys) clearBusy(key);
        }

        if (updaterKeys.length > 0) {
            try {
                const launchRes = await window.api?.updaterLaunchApply?.(updaterKeys);
                if (launchRes?.ok === false) {
                    setError(prev => (prev ? prev + '\n' : '') + (launchRes?.error || '업데이터 실행 실패'));
                }
            } catch (e) {
                setError(prev => (prev ? prev + '\n' : '') + `업데이터: ${e.message}`);
            }
        }

        await refreshStatus();

        if (hasDiscordBot && wasBotRunning) {
            try {
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
                    if (startRes?.error) {
                        setError(prev => (prev ? prev + '\n' : '') + `Discord Bot 재시작 실패: ${startRes.error}`);
                    }
                }
            } catch (e) {
                setError(prev => (prev ? prev + '\n' : '') + `Discord Bot 재시작: ${e.message}`);
            }
        }

        const summary = [];
        if (daemonApplied.length > 0) summary.push(`${daemonApplied.join(', ')} 적용 완료`);
        if (updaterKeys.length > 0) summary.push('업데이터로 적용 진행 중');
        if (summary.length > 0) setMessage(summary.join(' · '));
        else if (!error) setMessage('업데이트 처리 완료');
        setBusyAll(false);
    }, [components, refreshStatus]);

    // ── 모두 업데이트 ──
    const handleUpdateAll = useCallback(() => {
        const updatable = components.filter(c => c.update_available);
        if (updatable.length === 0) return;

        const hasNeedsUpdater = updatable.some(c => c.key === 'gui' || c.key === 'saba-core');
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

    // 패널 마운트 시 자동 확인 + mock 모드 감지
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
        handleCheck();
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
    }, [handleCheck]);

    // 자동 업데이트 확인 토글
    const handleAutoCheckToggle = useCallback(async (enabled) => {
        setAutoCheckEnabled(enabled);
        try {
            await window.api?.updaterSetConfig?.({ enabled });
        } catch (e) {
            console.error('Failed to set auto-check config:', e);
            setAutoCheckEnabled(!enabled); // rollback
        }
    }, []);

    const updatable = components.filter(c => c.update_available);
    const allUpToDate = components.length > 0 && updatable.length === 0;
    const anyBusy = checking || busyAll || busyKeys.size > 0;

    return (
        <div className={`update-panel ${isExiting ? 'exiting' : ''}`}>
            {/* 패널 헤더 — 뒤로가기 + 타이틀 + Mock 토글 */}
            <div className="update-panel-header">
                <button className="update-panel-back" onClick={onBack} title="뒤로" disabled={isExiting}>
                    <Icon name="chevronLeft" size="sm" />
                </button>
                <h2 className="update-panel-title">
                    {t('updates.modal_title', 'Update Center')}
                </h2>
                {devMode && (
                    <label className="update-mock-toggle" title={mockMode ? 'Mock 서버 (테스트)' : '실제 GitHub API'}>
                        <span className={`update-mock-label ${mockMode ? 'mock' : 'real'}`}>
                            {mockMode ? 'MOCK' : 'REAL'}
                        </span>
                        <SabaToggle
                            size="sm"
                            checked={!!mockMode}
                            onChange={(checked) => handleToggleMock(checked)}
                            disabled={mockMode === null || anyBusy}
                        />
                    </label>
                )}
            </div>

            {/* 상태 바 */}
            {checking && <div className="update-modal-status-bar"><Icon name="loader" size="sm" /> 업데이트 확인 중...</div>}
            {error && <div className="update-modal-status-bar error"><Icon name="xCircle" size="sm" /> {error}</div>}
            {message && <div className="update-modal-status-bar success"><Icon name="checkCircle" size="sm" /> {message}</div>}

            {/* 컴포넌트 리스트 */}
            {components.length > 0 && (
                <div className="update-modal-components">
                    {components.map(c => {
                        const isBusy = busyKeys.has(c.key) || busyAll;
                        let badgeClass = c.update_available ? 'available' : 'latest';
                        if (c.downloaded) badgeClass = 'downloaded';
                        const badge = c.update_available
                            ? (c.downloaded ? '다운로드됨' : '업데이트 가능')
                            : '최신';

                        return (
                            <div key={c.key} className={`update-modal-comp ${badgeClass}`}>
                                <Icon name={c.icon} size="sm" />
                                <div className="update-modal-comp-info">
                                    <span className="update-modal-comp-name">{c.display}</span>
                                    <span className="update-modal-comp-ver">
                                        v{c.current_version}
                                        {c.update_available && c.latest_version && (
                                            <> → <strong>v{c.latest_version}</strong></>
                                        )}
                                    </span>
                                </div>

                                <div className="update-modal-comp-actions">
                                    {c.update_available && !c.downloaded && (
                                        <button
                                            className="update-comp-btn download"
                                            disabled={isBusy}
                                            onClick={() => handleDownloadOne(c.key)}
                                            title="다운로드"
                                        >
                                            {isBusy
                                                ? <SabaSpinner size="xs" />
                                                : <Icon name="download" size="xs" />}
                                        </button>
                                    )}
                                    {c.update_available && c.downloaded && (
                                        <button
                                            className={`update-comp-btn apply ${c.needsUpdater ? 'warning' : ''}`}
                                            disabled={isBusy}
                                            onClick={() => handleApplyOne(c.key)}
                                            title={c.needsUpdater ? '적용 (재시작 필요)' : '적용'}
                                        >
                                            {isBusy
                                                ? <SabaSpinner size="xs" />
                                                : <Icon name={c.needsUpdater ? 'externalLink' : 'checkCircle'} size="xs" />}
                                        </button>
                                    )}
                                    {!c.update_available && (
                                        <span className={`update-modal-badge ${badgeClass}`}>{badge}</span>
                                    )}
                                </div>
                            </div>
                        );
                    })}
                </div>
            )}

            {/* 전체 최신 */}
            {allUpToDate && !message && (
                <div className="update-modal-status success">
                    <Icon name="checkCircle" size="sm" /> 모든 컴포넌트가 최신 버전입니다.
                </div>
            )}

            {/* 설정 */}
            <div className="update-panel-settings">
                <label className="update-panel-setting-row">
                    <span className="update-panel-setting-label">
                        <Icon name="refresh" size="sm" />
                        {t('updates.auto_check', '자동으로 업데이트 확인')}
                    </span>
                    <SabaToggle
                        checked={autoCheckEnabled}
                        onChange={(checked) => handleAutoCheckToggle(checked)}
                    />
                </label>
            </div>

            {/* 하단 액션 */}
            <div className="update-panel-actions">
                {updatable.length > 0 && (
                    <button
                        className="update-modal-btn primary"
                        disabled={anyBusy}
                        onClick={handleUpdateAll}
                    >
                        <Icon name="download" size="sm" /> 모두 업데이트
                    </button>
                )}
                <button className="update-modal-btn secondary" disabled={anyBusy} onClick={handleCheck}>
                    <Icon name="refresh" size="sm" /> 새로고침
                </button>
            </div>

            {/* 재시작 확인 모달 */}
            {confirmRestart && (
                <QuestionModal
                    title="재시작 필요"
                    message="업데이트를 적용하면 프로그램이 재시작됩니다. 계속하시겠습니까?"
                    onConfirm={handleConfirmRestart}
                    onCancel={() => { setConfirmRestart(false); setPendingRestartAction(null); }}
                />
            )}
        </div>
    );
}

export default UpdatePanel;
