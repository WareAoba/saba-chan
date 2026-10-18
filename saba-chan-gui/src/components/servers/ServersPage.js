import clsx from 'clsx';
import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useWindowSize } from '../../hooks/useWindowSize';
import { useServerStore } from '../../stores/useServerStore';
import { useSettingsStore } from '../../stores/useSettingsStore';
import { useUIStore } from '../../stores/useUIStore';
import { failedModuleName, failedModuleReason } from '../../utils/failedModules';
import { Icon } from '../Icon';
import { SystemMonitor, systemMonitorAvailable } from '../monitor/SystemMonitor';
import { DaemonDownNotice, DaemonDownState, daemonDown } from './DaemonDown';
import { OnboardingCard } from './OnboardingCard';
import { ServerDetail } from './ServerDetail';
import { ServerRow } from './ServerRow';

/** 이 너비보다 좁으면 목록/상세를 한 열로 (gui-design-reference §3) */
export const SINGLE_COLUMN_MAX_WIDTH = 900;
/** 이 너비 이상이면 시스템 모니터를 상세 오른쪽 열로, 아니면 상세 위에 둔다 */
export const MONITOR_SIDE_MIN_WIDTH = 1500;
const SEARCH_MIN_SERVERS = 6;

/**
 * 서버 페이지: 왼쪽 목록 + 오른쪽 상세 (master-detail).
 */
export function ServersPage({
    servers,
    setServers,
    modules,
    onAddServer,
    onStart,
    onStop,
    onRestart,
    onRename,
    onDelete,
    setModal,
    drag,
    consoleProps,
    settingsProps,
}) {
    const { t } = useTranslation('gui');
    const failedModules = useServerStore((st) => st.failedModules) || [];
    const { width } = useWindowSize();
    const singleColumn = width < SINGLE_COLUMN_MAX_WIDTH;
    const monitorOpen = useSettingsStore((s) => s.systemMonitorOpen);
    const monitorSide = monitorOpen && width >= MONITOR_SIDE_MIN_WIDTH;
    const selectedId = useUIStore((s) => s.activeServerId);
    const setSelectedId = useUIStore((s) => s.setActiveServerId);
    const setContextMenu = useUIStore((s) => s.setContextMenu);
    const isDaemonDown = useUIStore((s) => daemonDown(s.backgroundDaemonStatus));
    const [query, setQuery] = useState('');
    const [showDetailNarrow, setShowDetailNarrow] = useState(false);

    // 선택이 비었거나 사라졌으면 첫 서버를 고른다
    useEffect(() => {
        if (servers.length === 0) return;
        if (!servers.some((s) => s.id === selectedId)) setSelectedId(servers[0].id);
    }, [servers, selectedId, setSelectedId]);

    const moduleOf = useMemo(() => {
        const map = Object.fromEntries(modules.map((m) => [m.name, m]));
        return (name) => map[name];
    }, [modules]);

    const visible = query.trim()
        ? servers.filter((s) => `${s.name} ${s.module}`.toLowerCase().includes(query.trim().toLowerCase()))
        : servers;
    const selected = servers.find((s) => s.id === selectedId) || null;

    // 데몬이 꺼져 있으면 목록이 비어 보여도 서버가 없는 게 아니다
    if (servers.length === 0 && isDaemonDown) return <DaemonDownState />;

    if (servers.length === 0) {
        return (
            <div className="page page-empty">
                <div className="empty-state">
                    <img src="./icon.png" alt="" className="empty-mascot" />
                    <h2>{t('shell.empty_title')}</h2>
                    <p>{t('shell.empty_body')}</p>
                    <button type="button" className="btn btn-primary btn-lg" onClick={onAddServer}>
                        <Icon name="plus" size="sm" /> {t('shell.add_server')}
                    </button>
                </div>
            </div>
        );
    }

    const listHidden = singleColumn && showDetailNarrow && selected;
    const detailHidden = singleColumn && !showDetailNarrow;

    return (
        <div className={clsx('page servers-page', { 'single-column': singleColumn })}>
            {!listHidden && (
                <aside className="server-list-pane" aria-label={t('shell.nav_servers')}>
                    <div className="server-list-head">
                        <button type="button" className="btn btn-primary btn-block" onClick={onAddServer}>
                            <Icon name="plus" size="sm" /> {t('shell.add_server')}
                        </button>
                        {servers.length >= SEARCH_MIN_SERVERS && (
                            <input
                                type="search"
                                className="input"
                                value={query}
                                onChange={(e) => setQuery(e.target.value)}
                                placeholder={t('shell.search_placeholder')}
                                aria-label={t('shell.search_placeholder')}
                            />
                        )}
                    </div>
                    {isDaemonDown && <DaemonDownNotice />}
                    <OnboardingCard onAddServer={onAddServer} refreshKey={servers.length} />
                    {failedModules.length > 0 && (
                        // 모듈이 목록에서 사라진 이유를 알려준다 (module.toml 오류 등)
                        <details className="notice tone-warn module-failures">
                            <summary>
                                <Icon name="alertTriangle" size="sm" />
                                {t('shell.modules_failed', { count: failedModules.length })}
                            </summary>
                            <ul>
                                {failedModules.map((f) => (
                                    <li key={f.path}>
                                        <strong>{failedModuleName(f)}</strong>
                                        <span>{failedModuleReason(f, t)}</span>
                                    </li>
                                ))}
                            </ul>
                        </details>
                    )}
                    <div className="server-list" role="listbox" aria-label={t('shell.nav_servers')}>
                        {visible.length === 0 && <p className="muted pad">{t('shell.no_match')}</p>}
                        {visible.map((server) => (
                            <ServerRow
                                key={server.id || server.name}
                                server={server}
                                index={servers.indexOf(server)}
                                moduleData={moduleOf(server.module)}
                                selected={server.id === selectedId}
                                onSelect={(id) => {
                                    setSelectedId(id);
                                    setShowDetailNarrow(true);
                                }}
                                onStart={onStart}
                                onStop={onStop}
                                rowRef={(el) => {
                                    drag.cardRefs.current[server.name] = el;
                                }}
                                dragging={drag.draggedName === server.name}
                                onPointerDown={query ? () => {} : drag.handleCardPointerDown}
                                skipNextClick={drag.skipNextClick}
                                onContextMenu={(e) => {
                                    e.preventDefault();
                                    setContextMenu({ x: e.clientX, y: e.clientY, server });
                                }}
                            />
                        ))}
                    </div>
                </aside>
            )}
            {!detailHidden && selected && (
                <main className="server-detail-pane">
                    {isDaemonDown && listHidden && <DaemonDownNotice />}
                    {!monitorSide && systemMonitorAvailable() && (
                        <div className="monitor-top-slot">
                            <SystemMonitor layout="top" servers={servers} />
                        </div>
                    )}
                    <ServerDetail
                        server={selected}
                        moduleData={moduleOf(selected.module)}
                        modules={modules}
                        servers={servers}
                        setServers={setServers}
                        onStart={onStart}
                        onStop={onStop}
                        onRestart={onRestart}
                        onRename={onRename}
                        onDelete={onDelete}
                        onBack={singleColumn ? () => setShowDetailNarrow(false) : null}
                        setModal={setModal}
                        consoleProps={consoleProps}
                        settingsProps={settingsProps}
                    />
                </main>
            )}
            {!detailHidden && selected && monitorSide && (
                <aside className="monitor-pane">
                    <SystemMonitor layout="side" servers={servers} />
                </aside>
            )}
        </div>
    );
}
