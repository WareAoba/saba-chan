import clsx from 'clsx';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSettingsStore } from '../stores/useSettingsStore';
import { ExtensionSlot, Icon } from './index';
import { NativeProvision } from './NativeProvision';

/**
 * ServerCard — Individual server instance card with status, actions, and details.
 */
export function ServerCard({
    server,
    index,
    modules,
    servers,
    cardRefs,
    draggedName,
    skipNextClick,
    consoleServer,
    isConsoleOpen,
    consolePopoutInstanceId,
    handleCardPointerDown,
    handleStart,
    handleStop,
    handleOpenSettings,
    handleDeleteServer,
    openConsole,
    closeConsole,
    setCommandServer,
    setShowCommandModal,
    setServers,
    formatUptime,
    onContextMenu,
}) {
    const { t } = useTranslation('gui');
    const portConflictCheck = useSettingsStore((s) => s.portConflictCheck);
    const [provisionProgress, setProvisionProgress] = useState(null);
    const [updateInfo, setUpdateInfo] = useState(null);
    const [updatingServer, setUpdatingServer] = useState(false);

    // SteamCMD 인스턴스의 게임 서버 업데이트 확인 (3시간 주기, 무조건 체크)
    useEffect(() => {
        if (server.provisioning) return;
        if (!server.extension_data?.install_method_steamcmd) return;
        if (!window.api?.instanceCheckUpdate) return;

        let cancelled = false;
        const STEAM_UPDATE_CHECK_INTERVAL_MS = 3 * 60 * 60 * 1000; // 3시간

        const doCheck = async () => {
            try {
                const result = await window.api.instanceCheckUpdate(server.id);
                if (!cancelled && result && result.update_available) {
                    setUpdateInfo(result);
                } else if (!cancelled && result && !result.update_available) {
                    setUpdateInfo(null);
                }
            } catch {
                // ignore — background check
            }
        };

        doCheck(); // 즉시 1회 체크
        const timer = setInterval(doCheck, STEAM_UPDATE_CHECK_INTERVAL_MS);
        return () => { cancelled = true; clearInterval(timer); };
    }, [server.id, server.provisioning, server.extension_data?.install_method_steamcmd]);

    const handleApplyUpdate = async () => {
        if (!window.api?.instanceApplyUpdate) return;
        setUpdatingServer(true);
        try {
            const result = await window.api.instanceApplyUpdate(server.id);
            if (result?.success) {
                setUpdateInfo(null);
                // provisioning 상태로 전환 → 서버 목록 새로고침 시 반영
                setServers((prev) =>
                    prev.map((s) =>
                        s.id === server.id ? { ...s, provisioning: true } : s,
                    ),
                );
            }
        } catch {
            // ignore
        } finally {
            setUpdatingServer(false);
        }
    };

    // 프로비저닝 상태 폴링 — server.provisioning이 true인 동안 폴링
    useEffect(() => {
        if (!server.provisioning) {
            // tracker가 제거되면 (성공 후 auto-cleanup 또는 dismiss) UI 정리
            setProvisionProgress(null);
            return;
        }
        let cancelled = false;
        const poll = async () => {
            while (!cancelled) {
                try {
                    const result = await window.api.instanceProvisionProgress(server.name);
                    if (cancelled) break;
                    if (result && result.active) {
                        setProvisionProgress(result);
                        if (result.done) break; // done이면 폴링 중단 (UI는 유지)
                    }
                } catch {
                    // ignore
                }
                await new Promise((r) => setTimeout(r, 1200));
            }
        };
        poll();
        return () => {
            cancelled = true;
        };
    }, [server.provisioning, server.name]);

    const handleDismissProvision = async () => {
        try {
            const result = await window.api.instanceDismissProvision(server.name);
            setProvisionProgress(null);
            // 프로비저닝 실패로 인스턴스가 롤백(삭제)됐으면 목록에서 즉시 제거
            if (result?.rolled_back) {
                setServers((prev) => prev.filter((s) => s.name !== server.name));
            }
        } catch {
            /* ignore */
        }
    };

    const moduleData = modules.find((m) => m.name === server.module);
    const moduleMissing = !moduleData;
    const gameName = t(`mod_${server.module}:module.display_name`, {
        defaultValue: moduleData?.game_name || server.module,
    });
    const gameIcon = moduleData?.icon || null;

    return (
        <div
            ref={(el) => {
                cardRefs.current[server.name] = el;
            }}
            className={clsx('server-card', { expanded: server.expanded, dragging: draggedName === server.name, 'module-missing': moduleMissing })}
            onPointerDown={(e) => handleCardPointerDown(e, index)}
            onContextMenu={onContextMenu}
        >
            <div
                className="server-card-header"
                onClick={(e) => {
                    if (skipNextClick.current) return;
                    if (e.target.closest('button')) return;
                    setServers((prev) =>
                        prev.map((s) => (s.name === server.name ? { ...s, expanded: !s.expanded } : s)),
                    );
                }}
                style={{ cursor: 'pointer' }}
            >
                <div className="game-icon-container">
                    {gameIcon ? (
                        <img src={gameIcon} alt={gameName} className="game-icon" />
                    ) : (
                        <div className="game-icon-placeholder">
                            <Icon name="gamepad" size="lg" />
                        </div>
                    )}
                    <ExtensionSlot slotId="ServerCard.badge" server={server} />
                    {updateInfo && (
                        <span
                            className="server-update-badge"
                            title={t('server_status.update_available', {
                                local: updateInfo.local_buildid,
                                remote: updateInfo.remote_buildid,
                                defaultValue: `Update available (${updateInfo.local_buildid} → ${updateInfo.remote_buildid})`,
                            })}
                        >
                            <Icon name="arrowUp" size={14} />
                        </span>
                    )}
                    {portConflictCheck && server.port_conflicts && server.port_conflicts.length > 0 && (
                        <span
                            className="port-conflict-badge"
                            title={
                                t('errors.port_conflict') +
                                ': ' +
                                server.port_conflicts
                                    .map((c) =>
                                        t('errors.port_conflict_detail', { port: c.port, name: c.conflict_name }),
                                    )
                                    .join(', ')
                            }
                        >
                            <Icon name="alertCircle" size={16} />
                        </span>
                    )}
                    {(() => {
                        const sameModuleOthers = servers
                            ? servers.filter((s) => s.module === server.module && s.id !== server.id)
                            : [];
                        if (sameModuleOthers.length === 0) return null;
                        return (
                            <span
                                className="alias-conflict-badge"
                                title={t('errors.alias_ambiguity_card', {
                                    module: server.module,
                                    count: sameModuleOthers.length + 1,
                                    defaultValue: `Module '${server.module}' has ${sameModuleOthers.length + 1} instances — Discord alias is ambiguous`,
                                })}
                            >
                                <Icon name="copy" size={14} />
                            </span>
                        );
                    })()}
                </div>

                <div className="server-card-info">
                    <h2>{server.name}</h2>
                    <p className="game-name">
                        {gameName}
                        {server.server_version && <span className="server-version-badge">{server.server_version}</span>}
                        {server.id && (
                            <span className="instance-id-badge" title={server.id}>
                                {server.id.slice(0, 8)}
                            </span>
                        )}
                    </p>
                </div>

                {/* 익스텐션 제공 헤더 게이지 (예: Docker 메모리) */}
                <ExtensionSlot slotId="ServerCard.headerGauge" server={server} />

                {moduleMissing ? (
                    <span className="status-button status-module-missing" title={t('server_status.module_missing', { defaultValue: 'Module not found' })}>
                        <span className="status-label">
                            <Icon name="alertCircle" size="sm" />{' '}
                            {t('server_status.module_missing', { defaultValue: 'Module not found' })}
                        </span>
                        <span className="status-dot"></span>
                    </span>
                ) : server.provisioning ? (
                    <span className="status-button status-provisioning" title={t('server_status.provisioning', { defaultValue: 'Provisioning' })}>
                        <span className="status-label">
                            <Icon name="refresh" size="sm" className="spin" />{' '}
                            {t('server_status.provisioning', { defaultValue: 'Provisioning' })}
                        </span>
                        <span className="status-dot"></span>
                    </span>
                ) : updateInfo && server.status !== 'running' ? (
                    <button
                        className="status-button status-update-available"
                        onClick={handleApplyUpdate}
                        disabled={updatingServer}
                        title={t('server_status.update_available', {
                            local: updateInfo.local_buildid,
                            remote: updateInfo.remote_buildid,
                            defaultValue: `Update available (${updateInfo.local_buildid} → ${updateInfo.remote_buildid})`,
                        })}
                    >
                        <span className="status-label">
                            <Icon name="arrowUp" size="sm" />{' '}
                            {t('server_actions.update', { defaultValue: 'Update' })}
                        </span>
                        <span className="status-dot"></span>
                    </button>
                ) : (
                    <button
                        className={clsx('status-button', `status-${server.status}`)}
                        onClick={() => {
                            if (server.status === 'starting' || server.status === 'stopping') return;
                            if (server.status === 'running') handleStop(server.name);
                            else handleStart(server.name, server.module);
                        }}
                        disabled={server.status === 'starting' || server.status === 'stopping'}
                        title={
                            server.status === 'running' || server.status === 'starting'
                                ? t('server_actions.click_to_stop')
                                : t('server_actions.click_to_start')
                        }
                    >
                        <span className="status-label status-label-default">
                            {server.status === 'running'
                                ? t('server_status.running')
                                : server.status === 'starting'
                                  ? t('server_status.starting', { defaultValue: 'Starting' })
                                  : server.status === 'stopping'
                                    ? t('server_status.stopping')
                                    : t('server_status.stopped')}
                        </span>
                        <span className="status-label status-label-hover">
                            {server.status === 'running'
                                ? t('server_status.stop')
                                : server.status === 'starting'
                                  ? t('server_status.starting', { defaultValue: 'Starting' })
                                  : server.status === 'stopping'
                                    ? t('server_status.stopping')
                                    : t('server_status.start')}
                        </span>
                        <span className="status-dot"></span>
                    </button>
                )}
            </div>

            {/* -- 프로비저닝 진행 상태 -- */}
            {server.provisioning && (
                <>
                    {/* 익스텐션 제공 UI (Docker 등) */}
                    <ExtensionSlot
                        slotId="ServerCard.provision"
                        server={server}
                        provisionProgress={provisionProgress}
                        onDismiss={handleDismissProvision}
                        t={t}
                    />
                    {/* 네이티브 프로비저닝 UI (SteamCMD / download — 비-Docker) */}
                    <NativeProvision
                        server={server}
                        provisionProgress={provisionProgress}
                        onDismiss={handleDismissProvision}
                        t={t}
                    />
                </>
            )}

            <div className="server-card-collapsible">
                {moduleMissing && (
                    <div className="module-missing-banner">
                        <Icon name="alertTriangle" size="sm" />
                        <span>{t('server_status.module_missing_detail', {
                            module: server.module,
                            defaultValue: `Module '${server.module}' could not be found. Install the module or remove this instance.`,
                        })}</span>
                    </div>
                )}
                {!server.provisioning && !moduleMissing && (
                    <>
                        <div className="server-details">
                            {/* 익스텐션 제공 확장 통계 (예: Docker CPU/메모리 게이지) */}
                            <ExtensionSlot slotId="ServerCard.expandedStats" server={server} t={t} />
                            {/* 포트 충돌 경고 배너 */}
                            {portConflictCheck && server.port_conflicts && server.port_conflicts.length > 0 && (
                                <div className="port-conflict-banner">
                                    <Icon name="alertCircle" size="sm" />
                                    <div className="port-conflict-banner-content">
                                        <strong>{t('errors.port_conflict')}</strong>
                                        {server.port_conflicts.map((c, i) => (
                                            <div key={i} className="port-conflict-banner-detail">
                                                {t('errors.port_conflict_detail', {
                                                    port: c.port,
                                                    name: c.conflict_name,
                                                })}
                                            </div>
                                        ))}
                                    </div>
                                </div>
                            )}
                            {server.status === 'running' && server.pid && (
                                <div className="detail-row">
                                    <span className="label">PID:</span>
                                    <span className="value">{server.pid}</span>
                                </div>
                            )}
                            {server.status === 'running' && server.start_time && (
                                <div className="detail-row">
                                    <span className="label">{t('servers.uptime', 'Uptime')}:</span>
                                    <span className="value">{formatUptime(server.start_time)}</span>
                                </div>
                            )}
                            {server.port && (
                                <div className="detail-row">
                                    <span className="label">{t('servers.port', 'Port')}:</span>
                                    <span className="value">{server.port}</span>
                                </div>
                            )}
                            {server.rcon_port && (() => {
                                const mod = modules.find((m) => m.name === server.module);
                                const supported = mod?.protocols?.supported || [];
                                return supported.includes('rcon');
                            })() && (
                                <div className="detail-row">
                                    <span className="label">RCON:</span>
                                    <span className="value">{server.rcon_port}</span>
                                </div>
                            )}
                            {server.rest_port && (
                                <div className="detail-row">
                                    <span className="label">REST:</span>
                                    <span className="value">
                                        {server.rest_host || '127.0.0.1'}:{server.rest_port}
                                    </span>
                                </div>
                            )}
                            <div className="detail-row">
                                <span className="label">{t('servers.protocol', 'Protocol')}:</span>
                                <span className="value">
                                    {(() => {
                                        const mod = modules.find((m) => m.name === server.module);
                                        const proto = server.protocol_mode;
                                        if (proto === 'auto' || proto === 'rest') {
                                            const moduleDefault = mod?.protocols?.default;
                                            const supported = mod?.protocols?.supported || [];
                                            if (proto === 'rest' && supported.includes('rest')) return 'REST';
                                            if (moduleDefault) return moduleDefault.toUpperCase();
                                            if (supported.length > 0) return supported[0].toUpperCase();
                                        }
                                        return proto?.toUpperCase() || 'AUTO';
                                    })()}
                                </span>
                            </div>
                        </div>

                        <div className="server-actions">
                            <button className="action-icon" onClick={() => handleOpenSettings(server)} title={t('context_menu.settings')} disabled={moduleMissing}>
                                <Icon name="settings" size="md" />
                            </button>
                            {server.status === 'running' ? (
                                <>
                                    {(() => {
                                        const mod = modules.find((m) => m.name === server.module);
                                        const moduleMode = mod?.interaction_mode || 'console';
                                        const instanceManaged = server.module_settings?.managed_start;
                                        // 인스턴스별 managed_start 설정 우선, 없으면 모듈 기본값
                                        const isManaged = instanceManaged === true || (instanceManaged == null && moduleMode === 'console');
                                        const supported = mod?.protocols?.supported || [];
                                        const hasStdin = supported.includes('stdin');
                                        const hasCommands = (mod?.commands?.fields || []).length > 0;
                                        // managed 인스턴스는 콘솔 버튼만, 그 외는 기존 로직
                                        const showConsole = isManaged && hasStdin;
                                        const showCommand = !isManaged && (hasCommands || !showConsole);
                                        return (
                                            <>
                                                {showConsole && (() => {
                                                    const isPopoutActive = consolePopoutInstanceId === server.id;
                                                    const isOpen = isConsoleOpen ? isConsoleOpen(server.id) : (consoleServer?.id === server.id);
                                                    return (
                                                        <button
                                                            className={clsx('action-icon', {
                                                                'action-active':
                                                                    isOpen || isPopoutActive,
                                                            })}
                                                            onClick={async () => {
                                                                if (isPopoutActive) {
                                                                    try {
                                                                        await window.api.consoleFocusPopout(server.id);
                                                                    } catch (err) {
                                                                        console.error('[ServerCard] consoleFocusPopout failed:', err.message);
                                                                    }
                                                                    return;
                                                                }
                                                                if (isOpen) closeConsole(server.id);
                                                                else openConsole(server.id, server.name);
                                                            }}
                                                            title={t('server_actions.console')}
                                                        >
                                                            <Icon name="terminal" size="md" />
                                                        </button>
                                                    );
                                                })()}
                                                {showCommand && (
                                                    <button
                                                        className="action-icon"
                                                        onClick={() => {
                                                            setCommandServer(server);
                                                            setShowCommandModal(true);
                                                        }}
                                                        title={t('server_actions.command')}
                                                    >
                                                        <Icon name="command" size="md" />
                                                    </button>
                                                )}
                                            </>
                                        );
                                    })()}
                                </>
                            ) : (
                                <button
                                    className="action-icon action-delete"
                                    onClick={() => handleDeleteServer(server)}
                                    disabled={server.status === 'starting' || server.status === 'stopping'}
                                    title={t('context_menu.delete')}
                                >
                                    <Icon name="trash" size="md" />
                                </button>
                            )}
                        </div>
                    </>
                )}
            </div>
        </div>
    );
}
