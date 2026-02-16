import React from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from './index';

/**
 * ServerCard — Individual server instance card with status, actions, and details.
 */
export function ServerCard({
    server,
    index,
    modules,
    cardRefs,
    draggedName,
    skipNextClick,
    consoleServer,
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
    nowEpoch,
}) {
    const { t } = useTranslation('gui');

    const moduleData = modules.find(m => m.name === server.module);
    const gameName = t(`mod_${server.module}:module.display_name`, { defaultValue: moduleData?.game_name || server.module });
    const gameIcon = moduleData?.icon || null;

    return (
        <div
            ref={el => { cardRefs.current[server.name] = el; }}
            className={`server-card ${server.expanded ? 'expanded' : ''} ${draggedName === server.name ? 'dragging' : ''}`}
            onPointerDown={(e) => handleCardPointerDown(e, index)}
        >
            <div
                className="server-card-header"
                onClick={(e) => {
                    if (skipNextClick.current) return;
                    if (e.target.closest('button')) return;
                    setServers(prev => prev.map(s =>
                        s.name === server.name ? { ...s, expanded: !s.expanded } : s
                    ));
                }}
                style={{ cursor: 'pointer' }}
            >
                <div className="game-icon-container">
                    {gameIcon ? (
                        <img src={gameIcon} alt={gameName} className="game-icon" />
                    ) : (
                        <div className="game-icon-placeholder"><Icon name="gamepad" size="lg" /></div>
                    )}
                </div>

                <div className="server-card-info">
                    <h2>{server.name}</h2>
                    <p className="game-name">
                        {gameName}
                        {server.server_version && <span className="server-version-badge">{server.server_version}</span>}
                    </p>
                </div>

                <button
                    className={`status-button status-${server.status}`}
                    onClick={() => {
                        if (server.status === 'starting' || server.status === 'stopping') return;
                        if (server.status === 'running' || server.status === 'starting') handleStop(server.name);
                        else handleStart(server.name, server.module);
                    }}
                    disabled={server.status === 'starting' || server.status === 'stopping'}
                    title={server.status === 'running' || server.status === 'starting' ? 'Click to stop' : 'Click to start'}
                >
                    <span className="status-label status-label-default">
                        {server.status === 'running' ? t('server_status.running') :
                         server.status === 'starting' ? t('server_status.stopping') :
                         server.status === 'stopping' ? t('server_status.stopping') : t('server_status.stopped')}
                    </span>
                    <span className="status-label status-label-hover">
                        {server.status === 'running' ? t('server_status.stop') :
                         server.status === 'starting' ? t('server_status.stopping') :
                         server.status === 'stopping' ? t('server_status.stopping') : t('server_status.start')}
                    </span>
                    <span className="status-dot"></span>
                </button>
            </div>

            <div className="server-card-collapsible">
                <div className="server-details">
                    {server.status === 'running' && server.pid && (
                        <div className="detail-row"><span className="label">PID:</span><span className="value">{server.pid}</span></div>
                    )}
                    {server.status === 'running' && server.start_time && (
                        <div className="detail-row"><span className="label">{t('servers.uptime', 'Uptime')}:</span><span className="value">{formatUptime(server.start_time)}</span></div>
                    )}
                    {server.port && (
                        <div className="detail-row"><span className="label">{t('servers.port', 'Port')}:</span><span className="value">{server.port}</span></div>
                    )}
                    {server.rcon_port && (
                        <div className="detail-row"><span className="label">RCON:</span><span className="value">{server.rcon_port}</span></div>
                    )}
                    {server.rest_port && (
                        <div className="detail-row"><span className="label">REST:</span><span className="value">{server.rest_host || '127.0.0.1'}:{server.rest_port}</span></div>
                    )}
                    <div className="detail-row">
                        <span className="label">{t('servers.protocol', 'Protocol')}:</span>
                        <span className="value">{(() => {
                            const mod = modules.find(m => m.name === server.module);
                            const proto = server.protocol_mode;
                            if (proto === 'auto' || proto === 'rest') {
                                const moduleDefault = mod?.protocols?.default;
                                const supported = mod?.protocols?.supported || [];
                                if (proto === 'rest' && supported.includes('rest')) return 'REST';
                                if (moduleDefault) return moduleDefault.toUpperCase();
                                if (supported.length > 0) return supported[0].toUpperCase();
                            }
                            return proto?.toUpperCase() || 'AUTO';
                        })()}</span>
                    </div>
                </div>

                <div className="server-actions">
                    <button className="action-icon" onClick={() => handleOpenSettings(server)} title="Settings">
                        <Icon name="settings" size="md" />
                    </button>
                    {server.status === 'running' ? (
                        <>
                            {(() => {
                                const mod = modules.find(m => m.name === server.module);
                                const mode = mod?.interaction_mode || 'console';
                                if (mode === 'console') {
                                    const isPopoutActive = consolePopoutInstanceId === server.id;
                                    return (
                                        <button
                                            className={`action-icon ${consoleServer?.id === server.id || isPopoutActive ? 'action-active' : ''}`}
                                            onClick={async () => {
                                                if (isPopoutActive) {
                                                    await window.api.consoleFocusPopout(server.id);
                                                    return;
                                                }
                                                if (consoleServer?.id === server.id) closeConsole();
                                                else openConsole(server.id, server.name);
                                            }}
                                            title="Console"
                                        >
                                            <Icon name="terminal" size="md" />
                                        </button>
                                    );
                                } else {
                                    return (
                                        <button
                                            className="action-icon"
                                            onClick={() => { setCommandServer(server); setShowCommandModal(true); }}
                                            title="Command"
                                        >
                                            <Icon name="command" size="md" />
                                        </button>
                                    );
                                }
                            })()}
                        </>
                    ) : (
                        <button
                            className="action-icon action-delete"
                            onClick={() => handleDeleteServer(server)}
                            disabled={server.status === 'starting' || server.status === 'stopping'}
                            title="Delete"
                        >
                            <Icon name="trash" size="md" />
                        </button>
                    )}
                </div>
            </div>
        </div>
    );
}
