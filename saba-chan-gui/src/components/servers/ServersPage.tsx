import clsx from 'clsx';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { useDragReorder } from '../../hooks/useDragReorder';
import { useSwapAnimation } from '../../hooks/useSwapAnimation';
import { useWindowSize } from '../../hooks/useWindowSize';
import { type Server, useServerStore } from '../../stores/useServerStore';
import { useSettingsStore } from '../../stores/useSettingsStore';
import { type ModalConfig, useUIStore } from '../../stores/useUIStore';
import type { ModuleInfo } from '../../types/daemon';
import { failedModuleName, failedModuleReason } from '../../utils/failedModules';
import { Icon } from '../Icon';
import { LinkedText } from '../ui/ScreenLink';
import { SabaArt } from '../ui/SabaArt';
import { SystemMonitor, systemMonitorAvailable } from '../monitor/SystemMonitor';
import { useDaemonTarget } from '../shell/DaemonSwitcher';
import { OtherDaemons } from '../shell/OtherDaemons';
import { DaemonDownNotice, DaemonDownState, daemonDown } from './DaemonDown';
import { FirstRunGuide } from '../guides/FirstRunGuide';
import { GuideCard } from '../guides/GuideCard';
import { useGuideStore } from '../guides/guides';
import { OnboardingCard } from './OnboardingCard';
import type { ServerConsoleProps, ServerSettingsProps } from './ServerDetail';
import { ServerDetail } from './ServerDetail';
import { ServerRow } from './ServerRow';

/** 이 너비보다 좁으면 목록/상세를 한 열로 (gui-design-reference §3) */
export const SINGLE_COLUMN_MAX_WIDTH = 900;
/** 이 너비 이상이면 시스템 모니터를 상세 오른쪽 열로, 아니면 상세 위에 둔다 */
export const MONITOR_SIDE_MIN_WIDTH = 1500;
const SEARCH_MIN_SERVERS = 6;

interface ServersPageProps {
    servers: Server[];
    setServers: (updater: (prev: Server[]) => Server[]) => void;
    modules: ModuleInfo[];
    onAddServer: () => void;
    onStart: (name: string, moduleName: string) => void;
    onStop: (name: string) => void;
    onRestart?: (name: string) => void;
    onRename?: (id: string, name: string) => Promise<boolean> | boolean;
    onDelete: (server: Server) => void;
    setModal?: (modal: ModalConfig | null) => void;
    drag: ReturnType<typeof useDragReorder>;
    consoleProps: ServerConsoleProps;
    settingsProps: ServerSettingsProps;
}

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
}: ServersPageProps) {
    const { t, i18n } = useTranslation('gui');
    const failedModules = useServerStore((st) => st.failedModules) || [];
    const { width } = useWindowSize();
    const singleColumn = width < SINGLE_COLUMN_MAX_WIDTH;
    const monitorOpen = useSettingsStore((s) => s.systemMonitorExpanded);
    const monitorSide = monitorOpen && width >= MONITOR_SIDE_MIN_WIDTH;
    // 합쳐 보기 (G1c): 다른 데몬의 서버를 목록 아래에 데몬별로
    const unified = useSettingsStore((s) => s.daemonView) === 'unified' && typeof window.api?.remoteServers === 'function';
    const daemonTarget = useDaemonTarget();
    const selectedId = useUIStore((s) => s.activeServerId);
    const setSelectedId = useUIStore((s) => s.setActiveServerId);
    const isDaemonDown = useUIStore((s) => daemonDown(s.backgroundDaemonStatus));
    const [query, setQuery] = useState('');
    const [showDetailNarrow, setShowDetailNarrow] = useState(false);
    // 바깥에서 연 서버는 좁은 창에서도 상세로
    const revealServerCount = useUIStore((s) => s.revealServerCount);
    useEffect(() => {
        if (revealServerCount > 0) setShowDetailNarrow(true);
    }, [revealServerCount]);

    // 처음 시작하기: 서버 수·켜진 서버 수가 바뀌면 다음 할 일을 다시 묻는다 (첫 시작·디스코드 연결을 반영)
    const runningCount = servers.filter((s) => s.status === 'running').length;
    const activeTab = useUIStore((s) => s.activeTab);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 서버 수·켜진 수가 바뀔 때 다시 묻는 것이 목적이다
    useEffect(() => {
        if (activeTab === 'servers') useGuideStore.getState().refreshOnboarding();
    }, [servers.length, runningCount, activeTab]);
    // 가이드 카드: 완료(서버·모듈·켜짐)와 언어가 바뀌면 다시 묻는다
    // biome-ignore lint/correctness/useExhaustiveDependencies: 바뀔 때 다시 묻는 것이 목적이다
    useEffect(() => {
        if (activeTab === 'servers') useGuideStore.getState().refreshGuides();
    }, [servers.length, runningCount, modules.length, activeTab, i18n.language]);
    const guideStep = useGuideStore((s) => s.step);
    // 가이드를 연 채(갈림길 등) 다른 곳에서 서버가 생기면 가이드를 접는다 (모듈 받기는 원래 하던 곳으로 이어 가므로 둔다)
    const serverCount = useRef(servers.length);
    useEffect(() => {
        const grew = servers.length > serverCount.current;
        serverCount.current = servers.length;
        const step = useGuideStore.getState().step;
        if (grew && step && step !== 'modules') useGuideStore.getState().setStep(null);
    }, [servers.length]);
    const firstRunClosed = useSettingsStore((s) => s.closedGuides.includes('firstRun'));
    const settingsReady = useSettingsStore((s) => s.settingsReady);
    const discordDone = useGuideStore((s) => s.onboarding?.steps.find((x) => x.id === 'discord')?.done);

    // 선택이 비었거나 사라졌으면 첫 서버를 고른다
    useEffect(() => {
        if (servers.length === 0) return;
        if (!servers.some((s) => s.id === selectedId)) setSelectedId(servers[0].id);
    }, [servers, selectedId, setSelectedId]);

    const moduleOf = useMemo(() => {
        const map = Object.fromEntries(modules.map((m) => [m.name, m]));
        return (name: string): ModuleInfo | undefined => map[name];
    }, [modules]);

    const visible = query.trim()
        ? servers.filter((s) => `${s.name} ${s.module}`.toLowerCase().includes(query.trim().toLowerCase()))
        : servers;
    const selected = servers.find((s) => s.id === selectedId) || null;
    // 다른 서버를 고르면 상세가 살짝 떠오르며 바뀐다
    const detailRef = useSwapAnimation(selectedId);

    // 데몬이 꺼져 있으면 목록이 비어 보여도 서버가 없는 게 아니다
    if (servers.length === 0 && isDaemonDown) return <DaemonDownState />;

    // 가이드를 열었거나, 서버가 없고 처음 시작하기를 닫지 않았으면 가이드 화면
    if (guideStep) return <FirstRunGuide step={guideStep} />;
    if (servers.length === 0 && settingsReady && !firstRunClosed) {
        return <FirstRunGuide step={discordDone ? 'fork' : 'welcome'} />;
    }

    if (servers.length === 0) {
        return (
            <div className={clsx('page page-empty', { 'with-other-daemons': unified })}>
                <div className="empty-state">
                    <SabaArt kind="cleaning" size={280} />
                    <h2>{t('shell.empty_title')}</h2>
                    <p>{t('shell.empty_body')}</p>
                    {modules.length === 0 && (
                        // 모듈이 하나도 없으면 서버를 만들 수 없다: 모듈을 받는 사바 스토리지로 안내한다
                        <p className="empty-no-modules">
                            <LinkedText i18nKey="shell.empty_no_modules" to="settings/sabaStorage/modules" />
                        </p>
                    )}
                    <button type="button" data-spot="server.add" className="btn btn-primary btn-lg" onClick={onAddServer}>
                        <Icon name="plus" size="sm" /> {t('shell.add_server')}
                    </button>
                </div>
                <OnboardingCard />
                <GuideCard />
                {/* 고른 데몬에 서버가 없어도 다른 데몬의 서버는 보인다 */}
                {unified && daemonTarget !== undefined && <OtherDaemons currentTarget={daemonTarget} />}
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
                        <button type="button" data-spot="server.add" className="btn btn-primary btn-block" onClick={onAddServer}>
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
                                onRestart={onRestart}
                                onDelete={onDelete}
                            />
                        ))}
                    </div>
                    {unified && daemonTarget !== undefined && <OtherDaemons currentTarget={daemonTarget} />}
                    <GuideCard />
                </aside>
            )}
            {!detailHidden && selected && (
                <main className="server-detail-pane server-detail-flex">
                    {isDaemonDown && listHidden && <DaemonDownNotice />}
                    {!monitorSide && systemMonitorAvailable() && (
                        <div className="monitor-top-slot">
                            <SystemMonitor layout="top" servers={servers} />
                        </div>
                    )}
                    <div ref={detailRef} className="server-detail-host">
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
                    </div>
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
