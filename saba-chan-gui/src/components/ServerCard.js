import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon, MemoryGauge } from './index';

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
    const [provisionProgress, setProvisionProgress] = useState(null);

    // 프로비저닝 중일 때 진행 상태 폴링
    useEffect(() => {
        if (!server.provisioning) {
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
                        if (result.done) break;
                    }
                } catch {
                    // ignore
                }
                await new Promise(r => setTimeout(r, 1200));
            }
        };
        poll();
        return () => { cancelled = true; };
    }, [server.provisioning, server.name]);

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
                    {server.use_docker && (
                        <span className="docker-badge" title="Docker">
                            <Icon name="dockerL" size={14} color="var(--docker-badge-color, #2496ed)" />
                        </span>
                    )}
                </div>

                <div className="server-card-info">
                    <h2>{server.name}</h2>
                    <p className="game-name">
                        {gameName}
                        {server.server_version && <span className="server-version-badge">{server.server_version}</span>}
                    </p>
                </div>

                {/* 미니 메모리 게이지 (헤더 — 항상 표시) */}
                {!server.provisioning && server.use_docker && server.status === 'running' && server.docker_memory_percent != null && (
                    <MemoryGauge percent={server.docker_memory_percent} size={44} compact
                        title={server.docker_memory_usage || `${Math.round(server.docker_memory_percent)}%`} />
                )}

                {server.provisioning ? (
                    <span className="status-button status-provisioning" title="Provisioning...">
                        <span className="status-label">
                            <Icon name="refresh" size="sm" className="spin" />
                            {' '}{t('server_status.provisioning', { defaultValue: 'Provisioning' })}
                        </span>
                        <span className="status-dot"></span>
                    </span>
                ) : (
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
                         server.status === 'starting' ? t('server_status.starting', { defaultValue: 'Starting' }) :
                         server.status === 'stopping' ? t('server_status.stopping') : t('server_status.stopped')}
                    </span>
                    <span className="status-label status-label-hover">
                        {server.status === 'running' ? t('server_status.stop') :
                         server.status === 'starting' ? t('server_status.starting', { defaultValue: 'Starting' }) :
                         server.status === 'stopping' ? t('server_status.stopping') : t('server_status.start')}
                    </span>
                    <span className="status-dot"></span>
                </button>
                )}
            </div>

            {/* -- 프로비저닝 진행 상태 (카드에 inline 표시) -- */}
            {server.provisioning && (
                <div className="sc-provision-wrap">
                    <div className="as-provision-steps">
                        {[
                            { key: 'docker_engine', label: t('add_server_modal.step_docker_engine', { defaultValue: 'Docker Engine' }) },
                            { key: 'steamcmd', label: t('add_server_modal.step_steamcmd', { defaultValue: 'Server Files' }) },
                            { key: 'compose', label: t('add_server_modal.step_compose', { defaultValue: 'Configuration' }) },
                        ].map((s, idx) => {
                            const currentStep = provisionProgress?.step ?? -1;
                            const isDone = provisionProgress?.done && !provisionProgress?.error;
                            let stepClass = 'pending';
                            if (isDone || idx < currentStep) stepClass = 'completed';
                            else if (idx === currentStep) stepClass = provisionProgress?.error ? 'error' : 'active';
                            return (
                                <div key={s.key} className={`as-step ${stepClass}`}>
                                    <div className="as-step-icon">
                                        {stepClass === 'completed' ? <Icon name="check" size="xs" /> :
                                         stepClass === 'active' ? <Icon name="refresh" size="xs" className="spin" /> :
                                         stepClass === 'error' ? <Icon name="alertCircle" size="xs" /> :
                                         <span className="as-step-num">{idx + 1}</span>}
                                    </div>
                                    <span className="as-step-label">{s.label}</span>
                                </div>
                            );
                        })}
                    </div>
                    <div className="as-provision-bar">
                        {provisionProgress?.percent != null && !provisionProgress?.done && !provisionProgress?.error ? (
                            <div className="as-provision-bar-fill determinate" style={{ width: `${provisionProgress.percent}%` }} />
                        ) : (
                            <div className={`as-provision-bar-fill ${provisionProgress?.error ? 'error' : provisionProgress?.done ? 'done' : 'indeterminate'}`} />
                        )}
                    </div>
                    {provisionProgress?.message && (
                        <p className="as-provision-message">
                            {provisionProgress.message}
                            {provisionProgress?.percent != null && !provisionProgress?.done && !provisionProgress?.error && (
                                <span className="as-provision-pct"> ({provisionProgress.percent}%)</span>
                            )}
                        </p>
                    )}
                </div>
            )}

            <div className="server-card-collapsible">
                {!server.provisioning && (
                <>
                <div className="server-details">
                    {/* Docker 리소스 게이지 (확장 시 전체 표시) */}
                    {server.use_docker && server.status === 'running' && server.docker_memory_percent != null && (
                        <div className="docker-stats-row">
                            <MemoryGauge
                                percent={server.docker_memory_percent}
                                usage={server.docker_memory_usage}
                                size={130}
                            />
                            {server.docker_cpu_percent != null && (
                                <div className="docker-cpu-label">
                                    <span className="label">CPU</span>
                                    <span className="value">{server.docker_cpu_percent.toFixed(1)}%</span>
                                </div>
                            )}
                        </div>
                    )}
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
                </>
                )}
            </div>
        </div>
    );
}
