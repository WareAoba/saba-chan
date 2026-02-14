import React, { useState, useCallback, useRef, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { useModalClose } from '../../hooks/useModalClose';
import { QuestionModal } from './Modals';

/**
 * 인앱 업데이트 센터
 *
 * 데몬 HTTP API를 통해 업데이트를 확인/다운로드/적용합니다.
 * - 모듈: 데몬이 직접 적용 (프로세스 중단 불필요)
 * - 데몬/GUI/CLI: 업데이터 exe를 스폰하여 파일 교체
 *
 * 각 컴포넌트마다 독립적인 다운로드/적용 버튼이 있으며,
 * 하단 "모두 업데이트" 버튼으로 전체 일괄 처리 가능합니다.
 */
function UpdateModal({ isOpen, onClose }) {
    const { t } = useTranslation('gui');
    const { isClosing, requestClose } = useModalClose(onClose);

    const [components, setComponents] = useState([]);
    const [checking, setChecking] = useState(false);
    const [busyKeys, setBusyKeys] = useState(new Set());   // 개별 컴포넌트 busy 상태
    const [busyAll, setBusyAll] = useState(false);          // 모두 업데이트 진행 중
    const [error, setError] = useState(null);
    const [message, setMessage] = useState(null);
    const [lastChecked, setLastChecked] = useState(null);
    const [confirmRestart, setConfirmRestart] = useState(false); // 재시작 확인 모달
    const [pendingRestartAction, setPendingRestartAction] = useState(null); // 확인 후 실행할 콜백
    const [mockMode, setMockMode] = useState(null); // null=로딩중, true=mock, false=real
    const pollRef = useRef(null);

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
            needsUpdater: key === 'gui' || key === 'core_daemon',  // GUI/CoreDaemon은 업데이터 exe 필요
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
            setLastChecked(null);
        } catch (e) {
            setError(`주소 전환 실패: ${e.message}`);
        }
    }, []);

    // ── 상태 새로고침 (캐시된 상태만 — check가 아님) ──
    const refreshStatus = useCallback(async () => {
        try {
            const res = await window.api?.updaterStatus?.();
            if (res?.ok) setComponents(parseComponents(res.components));
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
                setLastChecked(new Date());
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
    // - 모듈: 데몬이 직접 적용
    // - core_daemon / cli: 데몬 API → needs_updater 응답 시 updater exe 스폰 (GUI 종료 안 함)
    // - gui: 유저 확인 후 updater exe 스폰 + GUI 종료/재시작
    const handleApplyOne = useCallback(async (key) => {
        // GUI 자체 업데이트 → 재시작 확인 모달 표시
        if (key === 'gui') {
            setPendingRestartAction(() => async () => {
                markBusy(key);
                setError(null);
                setMessage(null);
                try {
                    // 데몬에 apply 요청 → needs_updater 응답 → pending manifest 보장
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

        // Discord Bot → 봇 중지 → 파일 교체 → 봇 재시작
        if (key === 'discord_bot') {
            markBusy(key);
            setError(null);
            setMessage(null);
            try {
                // 1) 봇이 실행 중이면 먼저 중지
                const botStatus = await window.api?.discordBotStatus?.();
                const wasBotRunning = botStatus === 'running';
                if (wasBotRunning) {
                    setMessage('Discord Bot 중지 중...');
                    await window.api?.discordBotStop?.();
                    // 프로세스가 완전히 종료될 때까지 대기
                    await new Promise(r => setTimeout(r, 2000));
                }

                // 2) 데몬 API로 파일 교체
                setMessage('Discord Bot 파일 교체 중...');
                const res = await window.api?.updaterApply?.([key]);
                if (res?.ok === false) {
                    const errDetail = res?.errors?.length > 0
                        ? res.errors.join('; ') : (res?.error || '적용 실패');
                    setError(`${key}: ${errDetail}`);
                } else if (res?.applied?.length > 0) {
                    setMessage(`${res.applied.join(', ')} 적용 완료`);

                    // 3) 봇이 실행 중이었다면 재시작
                    if (wasBotRunning) {
                        setMessage('Discord Bot 재시작 중...');
                        // 토큰은 settings에, prefix/aliases는 botConfig에 저장됨
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

        // 모듈 / core_daemon / cli → 데몬 API로 적용 시도
        markBusy(key);
        setError(null);
        setMessage(null);
        try {
            const res = await window.api?.updaterApply?.([key]);
            if (res?.ok === false) {
                // 에러 배열이 있으면 상세 표시, 없으면 단일 에러 메시지
                const errDetail = res?.errors?.length > 0
                    ? res.errors.join('; ')
                    : (res?.error || '적용 실패');
                setError(`${key}: ${errDetail}`);
            } else {
                // 데몬/CLI는 needs_updater → updater exe 스폰 (GUI 종료 안 함)
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
                    // fallback — 조건에 해당하지 않는 경우
                    setMessage(`${key} 적용 요청 완료`);
                }
            }
            await refreshStatus();
        } catch (e) {
            setError(`${key}: ${e.message}`);
        }
        clearBusy(key);
    }, [refreshStatus]);

    // ── 모두 업데이트 내부 실행 (확인 완료 후) ──
    const executeUpdateAll = useCallback(async () => {
        const updatable = components.filter(c => c.update_available);
        if (updatable.length === 0) return;

        setBusyAll(true);
        setError(null);
        setMessage(null);

        const allKeys = updatable.map(c => c.key);
        // GUI/CoreDaemon은 업데이터 exe로, 나머지(모듈/CLI/DiscordBot)는 데몬 API로 직접 적용
        const updaterTargets = ['gui', 'core_daemon'];
        const daemonKeys = allKeys.filter(k => !updaterTargets.includes(k));
        const updaterKeys = allKeys.filter(k => updaterTargets.includes(k));
        const hasDiscordBot = allKeys.includes('discord_bot');

        // 0) Discord Bot이 실행 중이면 먼저 중지
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

        // 1) 전체 다운로드
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

        // 2) 데몬 API로 적용 (모듈 + 데몬 + CLI)
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

        // 3) 업데이터 exe 스폰 (GUI + CoreDaemon)
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

        // 3-1) Discord Bot이 실행 중이었다면 재시작
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

        // 결과 요약 메시지
        const summary = [];
        if (daemonApplied.length > 0) summary.push(`${daemonApplied.join(', ')} 적용 완료`);
        if (updaterKeys.length > 0) summary.push('업데이터로 적용 진행 중');
        if (summary.length > 0) setMessage(summary.join(' · '));
        else if (!error) setMessage('업데이트 처리 완료');
        setBusyAll(false);
    }, [components, refreshStatus]);

    // ── 모두 업데이트 (GUI/CoreDaemon 포함 시 확인 모달) ──
    const handleUpdateAll = useCallback(() => {
        const updatable = components.filter(c => c.update_available);
        if (updatable.length === 0) return;

        const hasNeedsUpdater = updatable.some(c => c.key === 'gui' || c.key === 'core_daemon');
        if (hasNeedsUpdater) {
            setPendingRestartAction(() => () => executeUpdateAll());
            setConfirmRestart(true);
        } else {
            executeUpdateAll();
        }
    }, [components, executeUpdateAll]);

    // 확인 모달에서 "계속 진행" 클릭
    const handleConfirmRestart = useCallback(() => {
        setConfirmRestart(false);
        if (pendingRestartAction) {
            pendingRestartAction();
            setPendingRestartAction(null);
        }
    }, [pendingRestartAction]);

    // 모달 열릴 때 자동 확인 + mock 모드 감지
    useEffect(() => {
        if (!isOpen) return;
        // 현재 설정에서 mock 모드 여부 판별
        (async () => {
            try {
                const cfgRes = await window.api?.updaterGetConfig?.();
                const cfg = cfgRes?.config || cfgRes;
                const isMock = !!(cfg?.api_base_url && cfg.api_base_url.includes('127.0.0.1'));
                setMockMode(isMock);
            } catch (_) {
                setMockMode(false);
            }
        })();
        handleCheck();
        pollRef.current = setInterval(async () => {
            try {
                const res = await window.api?.updaterStatus?.();
                if (res?.ok) setComponents(parseComponents(res.components));
            } catch (_) {}
        }, 30000);
        return () => clearInterval(pollRef.current);
    }, [isOpen, handleCheck]);

    // 닫힐 때 초기화
    useEffect(() => {
        if (!isOpen) {
            setComponents([]);
            setError(null);
            setMessage(null);
            setConfirmRestart(false);
            setPendingRestartAction(null);
        }
    }, [isOpen]);

    if (!isOpen) return null;

    const updatable = components.filter(c => c.update_available);
    const allUpToDate = components.length > 0 && updatable.length === 0;
    const anyBusy = checking || busyAll || busyKeys.size > 0;

    return (
        <div className={`modal-overlay ${isClosing ? 'closing' : ''}`} onClick={requestClose}>
            <div className="modal update-modal" onClick={e => e.stopPropagation()}>
                {/* 헤더 */}
                <div className="update-modal-header">
                    <h2 className="modal-title">
                        <Icon name="package" size="sm" /> {t('updates.modal_title', 'Update Center')}
                    </h2>
                    <div className="update-modal-header-right">
                        {/* Mock/Real 토글 */}
                        <label className="update-mock-toggle" title={mockMode ? 'Mock 서버 (테스트)' : '실제 GitHub API'}>
                            <span className={`update-mock-label ${mockMode ? 'mock' : 'real'}`}>
                                {mockMode ? 'MOCK' : 'REAL'}
                            </span>
                            <input
                                type="checkbox"
                                checked={!!mockMode}
                                onChange={(e) => handleToggleMock(e.target.checked)}
                                disabled={mockMode === null || anyBusy}
                            />
                            <span className="update-mock-slider" />
                        </label>
                        <button className="update-modal-close" onClick={requestClose}>
                            <Icon name="close" size="sm" />
                        </button>
                    </div>
                </div>

                {/* 상태 바 */}
                {checking && <div className="update-modal-status-bar"><Icon name="loader" size="sm" /> 업데이트 확인 중...</div>}
                {error && <div className="update-modal-status-bar error"><Icon name="xCircle" size="sm" /> {error}</div>}
                {message && <div className="update-modal-status-bar success"><Icon name="checkCircle" size="sm" /> {message}</div>}

                {/* 컴포넌트 리스트 — 각 행에 독립 다운로드/적용 버튼 */}
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

                                    {/* 개별 액션 영역 */}
                                    <div className="update-modal-comp-actions">
                                        {c.update_available && !c.downloaded && (
                                            <button
                                                className="update-comp-btn download"
                                                disabled={isBusy}
                                                onClick={() => handleDownloadOne(c.key)}
                                                title="다운로드"
                                            >
                                                {isBusy
                                                    ? <span className="update-spinner" />
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
                                                    ? <span className="update-spinner" />
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

                {/* 하단 액션 — "모두 업데이트" + 새로고침 */}
                <div className="update-modal-actions">
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

                {/* 마지막 확인 시각 */}
                {lastChecked && (
                    <div className="update-modal-footer">
                        마지막 확인: {lastChecked.toLocaleTimeString('ko-KR', { hour12: false })}
                    </div>
                )}
            </div>

            {/* ── 재시작 확인 모달 (기존 QuestionModal 활용) ── */}
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

export default UpdateModal;
