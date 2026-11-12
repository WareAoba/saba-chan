import { type ComponentProps, type ReactNode, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
    applyInstanceUpdate,
    closeCookedCard,
    dismissInstanceProvision,
    useMaintenanceOf,
} from '../../hooks/useInstanceMaintenance';
import type { useMultiConsole } from '../../hooks/useMultiConsole';
import { useExtensions } from '../../contexts/ExtensionContext';
import { extensionInstallMethod } from '../../utils/installMethod';
import type { useServerSettings } from '../../hooks/useServerSettings';
import { useConsoleStore } from '../../stores/useConsoleStore';
import { type Server, useServerStore } from '../../stores/useServerStore';
import { useSettingsStore } from '../../stores/useSettingsStore';
import { failedModuleReason, failureForModule } from '../../utils/failedModules';
import { openWithNotice, safeShowToast } from '../../utils/helpers';
import { isImeKey } from '../../utils/ime';
import { screenFallbackNotice } from '../../utils/openScreen';
import { type ModalConfig, useUIStore } from '../../stores/useUIStore';
import type { ModuleInfo } from '../../types/daemon';
import { withCommandAliases } from '../../utils/commandRisk';
import {
    STATUS_TONE,
    isTransitional,
    buildDate,
    consoleQuickCommands,
    defaultServerTab,
    gameNameOf,
    initials,
    interactionOf,
    playersText,
    primaryAction,
    protocolLabel,
    viewStatus,
} from '../../utils/serverView';
import { dataStatusRows } from '../DataSlots';
import ExtensionSlot, { SlotView, useSlotPages } from '../ExtensionSlot';
import { Icon } from '../Icon';
import { PageFind, PageFindButton } from './PageFind';
import { PortCheck, PortCheckNotice } from './PortCheck';
import { ServerAddress } from './ServerAddress';
import { ServerCheck } from './ServerCheck';
import CommandModal from '../Modals/CommandModal';
import { NativeProvision } from '../NativeProvision';
import { Card, ServerSettingsModal, SettingRow } from '../ServerSettingsModal';
import { Disclosure } from '../ui/Disclosure';
import { Segmented, StatusChip } from '../ui/StatusChip';
import { ServerLogs } from './ServerLogs';
import { InstanceDiscordTab, instanceWordsAvailable } from '../discord/InstanceWordsTab';
import { MaintenanceTab, maintenanceAvailable } from './MaintenanceTab';
import { PlayersTab, playerListsAvailable } from './PlayersTab';
import { ModuleAuditNotice } from './ModuleAuditNotice';
import { ServerConsole } from './ServerConsole';
import { StartCoach } from './OnboardingCard';

type ServerSetter = (updater: (prev: Server[]) => Server[]) => void;

interface ServerDetailProps {
    server: Server;
    moduleData?: ModuleInfo;
    modules: ModuleInfo[];
    servers: Server[];
    setServers: ServerSetter;
    onStart: (name: string, moduleName: string) => void;
    onStop: (name: string) => void;
    onRestart?: (name: string) => void;
    onRename?: (id: string, name: string) => Promise<boolean> | boolean;
    onDelete: (server: Server) => void;
    onBack?: (() => void) | null;
    setModal?: (modal: ModalConfig | null) => void;
    consoleProps: ServerConsoleProps;
    settingsProps: ServerSettingsProps;
}

/** App이 서버 화면에 넘기는 콘솔 묶음 */
export type ServerConsoleProps = Pick<
    ReturnType<typeof useMultiConsole>,
    'openConsole' | 'watchConsole' | 'setConsoleInput' | 'sendConsoleCommand' | 'consolePopoutIds' | 'popinConsole'
>;

/** App이 서버 화면에 넘기는 설정 묶음: 닫기 상태는 isClosing 하나로 넘긴다 */
export type ServerSettingsProps = Omit<
    ReturnType<typeof useServerSettings>,
    'isSettingsClosing' | 'requestSettingsClose'
> & { isClosing: boolean };

/**
 * 오른쪽 서버 상세. 머리(이름·상태·켜기/끄기) + 탭(콘솔 · 명령 · 설정 · 백업·예약 · 정보).
 * 예전에는 콘솔·명령·설정이 각각 떠다니는 창/모달이었다.
 */
export function ServerDetail({
    server,
    moduleData,
    modules,
    servers,
    setServers,
    onStart,
    onStop,
    onRestart,
    onRename,
    onDelete,
    onBack,
    consoleProps,
    settingsProps,
}: ServerDetailProps) {
    const { t, i18n } = useTranslation('gui');
    const portConflictCheck = useSettingsStore((s) => s.portConflictCheck);
    const { provisionProgress, cooked, updateInfo, updatingServer } = useMaintenanceOf(server.id);
    const { extensions } = useExtensions();
    const modulesLoaded = useServerStore((s) => s.modulesLoaded);
    const status = viewStatus(server, moduleData, modulesLoaded);
    const moduleFailure = useServerStore((st) =>
        status === 'missing' ? failureForModule(st.failedModules, server.module) : null,
    );
    const action = primaryAction(status);
    // 설치 완성 카드의 '시작'은 서버가 켜지면 닫는다: 켜진 서버에 다시 시작을 보내지 않게 (사용자 11-06 #10)
    const live = status === 'running' || status === 'starting';
    // biome-ignore lint/correctness/useExhaustiveDependencies: 켜짐·완성 표시가 바뀔 때만
    useEffect(() => {
        if (cooked && live) closeCookedCard(server);
    }, [cooked, live]);
    const interaction = interactionOf(server, moduleData);
    const gameName = gameNameOf(t, server.module, moduleData);

    const extTabs = useSlotPages('ServerDetail.tab');
    const tabs = [
        interaction.console && {
            id: 'console',
            label: t('shell.tab_console'),
            icon: <Icon name="terminal" size="sm" />,
        },
        interaction.commands && {
            id: 'commands',
            label: t('shell.tab_commands'),
            icon: <Icon name="command" size="sm" />,
        },
        playerListsAvailable(moduleData) && {
            id: 'players',
            label: t('shell.tab_players'),
            icon: <Icon name="user" size="sm" />,
        },
        { id: 'settings', label: t('shell.tab_settings'), icon: <Icon name="gamepad" size="sm" /> },
        { id: 'saba', label: t('shell.tab_saba'), icon: <Icon name="sabaFace" size="sm" /> },
        maintenanceAvailable() && {
            id: 'maintenance',
            label: t('shell.tab_maintenance'),
            icon: <Icon name="clock" size="sm" />,
        },
        instanceWordsAvailable() && {
            id: 'discord',
            label: t('shell.tab_discord'),
            icon: <Icon name="discord" size="sm" />,
        },
        // 익스텐션 탭 (`ServerDetail.tab` slot): "정보" 바로 앞
        ...extTabs.map((p) => ({ id: p.id, label: p.label, icon: <Icon name={p.icon} size="sm" /> })),
        { id: 'info', label: t('shell.tab_info'), icon: <Icon name="info" size="sm" /> },
    ].filter(Boolean) as { id: string; label: string; icon: ReactNode }[];

    // null = 아직 고르지 않음 → 아래 defaultTab
    const [tab, setTab] = useState<string | null>(null);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 서버가 바뀔 때만 초기화
    useEffect(() => {
        setTab(null);
    }, [server.id]);
    // 우클릭 메뉴 등 바깥에서 특정 탭을 요청하면 그 탭으로
    const requestedTab = useUIStore((s) => s.requestedServerTab);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 요청이 올 때만 그때의 탭 목록으로 판단한다
    useEffect(() => {
        if (!requestedTab) return;
        // 이 서버에 없는 탭(화면 주소 링크)이면 기본 탭에 두고 이유를 알린다
        if (!tabs.some((x) => x.id === requestedTab)) screenFallbackNotice('tab_missing');
        else setTab(requestedTab);
        useUIStore.getState().clearRequestedServerTab();
    }, [requestedTab]);
    const defaultTab = defaultServerTab(interaction, server.status);
    const activeTab = tabs.some((x) => x.id === tab) ? (tab as string) : defaultTab;
    const activeExtTab = extTabs.find((p) => p.id === activeTab);
    // Ctrl+F 찾기는 칸이 많은 설정·사바쨩·디스코드 탭에서만 (사용자 11-10)
    const findable = ['settings', 'saba', 'discord'].includes(activeTab);
    const poppedOut = consoleProps.consolePopoutIds.includes(server.id);
    // 콘솔을 PIP 창으로 띄운다 (이미 떠 있으면 그 창을 앞으로). 창이 열리면 셸이 console:popoutOpened로 알려 준다. 못 열면 알린다 (G-C5)
    const popoutConsole = async () => {
        try {
            const res = await window.api.consolePopout(server.id, server.name);
            if (res?.ok === false) {
                console.error('[ServerDetail] popout failed:', res.error);
                safeShowToast(t('console.popout_failed'), 'error');
            }
        } catch (err) {
            console.error('[ServerDetail] popout failed:', err);
            safeShowToast(t('console.popout_failed'), 'error');
        }
    };
    const bodyRef = useRef<HTMLDivElement>(null);
    const tabRowRef = useRef<HTMLDivElement>(null);

    const conflicts = portConflictCheck ? server.port_conflicts || [] : [];
    const uptime = status === 'running' && server.start_time ? <Uptime startTime={server.start_time} /> : null;
    const players = playersText(server);
    // 설치 방식 익스텐션으로 받은 서버는 데몬이 적어 둔 빌드 번호가 버전이다
    const versionTag = server.server_build ? t('shell.build', { id: server.server_build.id }) : server.server_version;
    // 새 빌드를 알려 준 곳(설치 방식 익스텐션 이름)과 올라온 날
    const installMethod = extensionInstallMethod(server, moduleData);
    const updateFrom = [
        installMethod && extensions.find((e) => e.id === installMethod)?.name,
        buildDate(updateInfo?.remote_build_time, i18n.language),
    ]
        .filter(Boolean)
        .join(', ');

    const primaryButton = (() => {
        if (status === 'missing') return null;
        if (action === 'busy') {
            return (
                <button type="button" className="btn btn-ghost btn-lg" disabled>
                    <span className="spinner" aria-hidden="true" /> {t(`shell.status_${status}`)}
                </button>
            );
        }
        return action === 'stop' ? (
            <>
                {onRestart && typeof window.api?.instanceRestart === 'function' && (
                    <button
                        type="button"
                        className="icon-btn"
                        onClick={() => onRestart(server.name)}
                        data-tooltip={t('shell.restart')}
                        aria-label={t('shell.restart')}
                    >
                        <Icon name="refresh" size="sm" />
                    </button>
                )}
                <button type="button" className="btn btn-danger btn-lg" onClick={() => onStop(server.name)}>
                    <Icon name="stop" size="sm" /> {t('shell.stop')}
                </button>
            </>
        ) : (
            <span className="start-anchor">
                <button data-spot="server.start"
                    type="button"
                    className="btn btn-primary btn-lg"
                    onClick={() => onStart(server.name, server.module)}
                >
                    <Icon name="play" size="sm" /> {t('shell.start')}
                </button>
                <StartCoach />
            </span>
        );
    })();

    return (
        <section
            className="server-detail"
            aria-label={server.name}
        >
            <header className="detail-head">
                {onBack && (
                    <button
                        type="button"
                        className="icon-btn detail-back"
                        onClick={onBack}
                        aria-label={t('shell.back')}
                    >
                        <Icon name="chevronLeft" size="sm" />
                    </button>
                )}
                <div className="detail-icon">
                    {moduleData?.icon ? <img src={moduleData.icon} alt="" /> : <span>{initials(gameName)}</span>}
                </div>
                <div className="detail-title">
                    <ServerName server={server} canRename={!!onRename && status === 'stopped'} onRename={onRename} />
                    <div className="detail-sub">
                        <span>{gameName}</span>
                        {versionTag && <span className="tag">{versionTag}</span>}
                        {updateInfo && <span className="tag tag-accent">{t('shell.update_tag')}</span>}
                        {server.port && (
                            <span className="muted">
                                {t('shell.info_port')} {server.port}
                            </span>
                        )}
                        {players && (
                            <span className="muted">
                                {t('shell.players')} {players}
                            </span>
                        )}
                    </div>
                </div>
                <ExtensionSlot slotId="ServerCard.headerGauge" server={server} />
                <div className="detail-actions">
                    <StatusChip
                        tone={STATUS_TONE[status]}
                        label={t(`shell.status_${status}`)}
                        detail={uptime}
                        pulse={isTransitional(status)}
                    />
                    {primaryButton}
                </div>
            </header>

            {status === 'missing' && (
                <div className="notice tone-warn">
                    <Icon name="alertTriangle" size="sm" />
                    <span>
                        {t('server_status.module_missing_detail', {
                            module: server.module,
                            defaultValue: `Module '${server.module}' could not be found.`,
                        })}
                        {moduleFailure && (
                            <span className="notice-detail">
                                {t('shell.module_load_error', { error: failedModuleReason(moduleFailure, t) })}
                            </span>
                        )}
                    </span>
                </div>
            )}

            {status !== 'missing' && <ModuleAuditNotice moduleName={server.module} />}

            {server.unresponsive && status === 'running' && (
                <div className="notice tone-warn idle-stopped-notice" role="status">
                    <Icon name="alertTriangle" size="sm" />
                    <span>
                        {server.unresponsive_minutes != null && (
                            <>
                                <strong>
                                    {t('server_status.unresponsive_for', { minutes: server.unresponsive_minutes })}
                                </strong>{' '}
                                {t('server_status.unresponsive_detail')}{' '}
                            </>
                        )}
                        <span className="notice-detail">
                            {t('server_status.unresponsive', { reason: server.unresponsive })}
                        </span>
                    </span>
                    {onRestart && typeof window.api?.instanceRestart === 'function' && (
                        <button type="button" className="btn btn-ghost btn-sm" onClick={() => onRestart(server.name)}>
                            <Icon name="refresh" size="sm" /> {t('shell.restart')}
                        </button>
                    )}
                </div>
            )}

            {/* 새 빌드가 있다: 무엇에서 무엇으로 바뀌는지와 올라온 날을 보이고 여기서 업데이트한다 */}
            {updateInfo && status === 'stopped' && (
                <div className="notice tone-info idle-stopped-notice update-notice" role="status">
                    <Icon name="info" size="sm" />
                    <span>
                        {t('shell.update_notice', { from: updateInfo.local_buildid, to: updateInfo.remote_buildid })}
                        {updateFrom && ` (${updateFrom})`}
                    </span>
                    <button
                        type="button"
                        className="btn btn-primary btn-sm"
                        disabled={updatingServer}
                        onClick={() => applyInstanceUpdate(server, setServers)}
                    >
                        {updatingServer ? t('shell.updating') : t('shell.update')}
                    </button>
                </div>
            )}

            {/* 접속자가 없어 데몬이 자동으로 정지했다 (데몬 idle_stopped: 다시 켜면 사라진다) */}
            {server.idle_stopped && status === 'stopped' && (
                <div className="notice tone-info idle-stopped-notice" role="status">
                    <Icon name="moon" size="sm" />
                    <span>
                        <strong>{t('server_settings.idle_stopped', { minutes: server.idle_stopped.minutes })}</strong>{' '}
                        {new Date(server.idle_stopped.at * 1000).toLocaleString(i18n.language, {
                            month: 'numeric',
                            day: 'numeric',
                            hour: '2-digit',
                            minute: '2-digit',
                        })}
                    </span>
                    <button
                        type="button"
                        className="btn btn-primary btn-sm"
                        onClick={() => onStart(server.name, server.module)}
                    >
                        <Icon name="play" size="sm" /> {t('shell.start')}
                    </button>
                </div>
            )}

            {status !== 'missing' && <ServerAddress server={server} />}

            {status !== 'missing' && <PortCheckNotice server={server} />}

            {conflicts.length > 0 && (
                <div className="notice tone-warn">
                    <Icon name="alertCircle" size="sm" />
                    <span>
                        <strong>{t('errors.port_conflict')}</strong>{' '}
                        {conflicts
                            .map((c) => t('errors.port_conflict_detail', { port: c.port, name: c.conflict_name }))
                            .join(', ')}
                    </span>
                </div>
            )}

            {(server.provisioning || (cooked && !live)) && (
                <div className="detail-provision">
                    {server.provisioning && (
                        <ExtensionSlot
                            slotId="ServerCard.provision"
                            server={server}
                            provisionProgress={provisionProgress}
                            onDismiss={() => dismissInstanceProvision(server, setServers)}
                            t={t}
                        />
                    )}
                    <NativeProvision
                        server={server}
                        provisionProgress={provisionProgress as ComponentProps<typeof NativeProvision>['provisionProgress']}
                        cooked={cooked && !live}
                        icon={
                            moduleData?.icon ? <img src={moduleData.icon} alt="" /> : <span>{initials(gameName)}</span>
                        }
                        onDismiss={() => dismissInstanceProvision(server, setServers)}
                        onClose={() => closeCookedCard(server)}
                        onStart={() => onStart(server.name, server.module)}
                        onSettings={() => setTab('settings')}
                    />
                </div>
            )}

            {status !== 'missing' && !server.provisioning && (
                <>
                    <div className="detail-tabrow" ref={tabRowRef}>
                        <Segmented items={tabs} value={activeTab} onChange={setTab} ariaLabel={server.name} spot="server.tab" />
                        {findable && <PageFindButton />}
                        {activeTab === 'console' && !poppedOut && typeof window.api?.consolePopout === 'function' && (
                            <button
                                type="button"
                                data-spot="console.popout"
                                className="btn btn-ghost detail-pip-btn"
                                onClick={popoutConsole}
                            >
                                <Icon name="pictureInPicture" size={15} />
                                {t('console.popout')}
                            </button>
                        )}
                    </div>
                    <PageFind
                        rootRef={bodyRef}
                        anchorRef={tabRowRef}
                        enabled={findable}
                        pageKey={`${server.id}-${activeTab}`}
                    />
                    <div className="detail-body" key={`${server.id}-${activeTab}`} ref={bodyRef}>
                        {activeTab === 'console' && (
                            <LiveServerConsole
                                server={server}
                                readOnly={!interaction.input}
                                highlightRules={moduleData?.syntax_highlight?.rules || null}
                                quickCommands={consoleQuickCommands(moduleData?.commands?.fields, moduleData?.command_aliases)}
                                openConsole={consoleProps.openConsole}
                                watchConsole={consoleProps.watchConsole}
                                setConsoleInput={consoleProps.setConsoleInput}
                                sendConsoleCommand={consoleProps.sendConsoleCommand}
                                poppedOut={poppedOut}
                                onPopout={popoutConsole}
                                onPopin={() => consoleProps.popinConsole(server.id)}
                            />
                        )}
                        {activeTab === 'commands' &&
                            (server.status === 'running' ? (
                                <CommandModal server={server} modules={modules} />
                            ) : (
                                <div className="empty-inline">
                                    <Icon name="command" size="lg" />
                                    <p>{t('shell.commands_offline')}</p>
                                </div>
                            ))}
                        {activeTab === 'players' && <PlayersTab server={server} moduleData={moduleData} />}
                        {(activeTab === 'settings' || activeTab === 'saba') && (
                            <SettingsTab
                                section={activeTab === 'saba' ? 'saba' : 'server'}
                                server={server}
                                settingsProps={settingsProps}
                                modules={modules}
                                servers={servers}
                            />
                        )}
                        {activeTab === 'maintenance' && (
                            <MaintenanceTab
                                server={server}
                                moduleCommands={withCommandAliases(
                                    moduleData?.commands?.fields,
                                    moduleData?.command_aliases,
                                )}
                            />
                        )}
                        {activeTab === 'discord' && <InstanceDiscordTab server={server} />}
                        {activeExtTab && (
                            <SlotView entry={activeExtTab.entry} slotId="ServerDetail.tab" server={server} t={t} />
                        )}
                        {activeTab === 'info' && (
                            <InfoTab
                                server={server}
                                moduleData={moduleData}
                                gameName={gameName}
                                uptime={uptime}
                                onDelete={onDelete}
                            />
                        )}
                    </div>
                </>
            )}
            {(status === 'missing' || server.provisioning) && (
                <InfoTab
                    server={server}
                    moduleData={moduleData}
                    gameName={gameName}
                    uptime={uptime}
                    onDelete={onDelete}
                />
            )}
        </section>
    );
}

/**
 * 서버 이름. 꺼져 있을 때 연필을 누르면 그 자리에서 고친다 (Enter 저장, Esc 취소).
 * 켜진 서버는 데몬이 이름 바꾸기를 거절하므로 연필을 숨긴다.
 */
function ServerName({
    server,
    canRename,
    onRename,
}: {
    server: Server;
    canRename: boolean;
    onRename?: ServerDetailProps['onRename'];
}) {
    const { t } = useTranslation('gui');
    const [editing, setEditing] = useState(false);
    const [value, setValue] = useState(server.name);
    const [saving, setSaving] = useState(false);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 다른 서버로 바뀌면 편집을 끝낸다
    useEffect(() => {
        setEditing(false);
        setValue(server.name);
    }, [server.id, server.name]);

    if (!editing) {
        return (
            <h1 className="detail-name">
                <span>{server.name}</span>
                {canRename && typeof window.api?.instanceRename === 'function' && (
                    <button
                        type="button"
                        className="icon-btn detail-rename"
                        onClick={() => setEditing(true)}
                        data-tooltip={t('shell.rename')}
                        aria-label={t('shell.rename')}
                    >
                        <Icon name="edit" size="sm" />
                    </button>
                )}
            </h1>
        );
    }
    const commit = async () => {
        if (saving) return;
        const next = value.trim();
        if (!next || next === server.name) {
            setEditing(false);
            setValue(server.name);
            return;
        }
        setSaving(true);
        const ok = await (onRename as NonNullable<typeof onRename>)(server.id, next);
        setSaving(false);
        if (ok) setEditing(false);
    };
    return (
        <input
            className="input detail-name-input"
            value={value}
            disabled={saving}
            aria-label={t('shell.rename')}
            // biome-ignore lint/a11y/noAutofocus: 연필을 누른 직후 바로 입력하게
            autoFocus
            onChange={(e) => setValue(e.target.value)}
            onBlur={commit}
            onKeyDown={(e) => {
                if (e.key === 'Enter' && !isImeKey(e)) e.currentTarget.blur();
                if (e.key === 'Escape') {
                    setValue(server.name);
                    setEditing(false);
                }
            }}
        />
    );
}

/** 설정·사바쨩 탭: 예전 "서버 설정" 모달 내용을 게임 서버 설정(server)과 사바쨩 설정(saba)으로 나눠 박아 넣는다 */
function SettingsTab({
    section,
    server,
    settingsProps,
    modules,
    servers,
}: {
    section: 'server' | 'saba';
    server: Server;
    settingsProps: ServerDetailProps['settingsProps'];
    modules: ModuleInfo[];
    servers: Server[];
}) {
    const { t } = useTranslation('gui');
    const { showSettingsModal, settingsServer, handleOpenSettings } = settingsProps;
    const ready = showSettingsModal && settingsServer?.id === server.id;

    // 탭을 열거나 설정 창이 닫히면(버전 설치 등) 최신 값으로 다시 연다
    // biome-ignore lint/correctness/useExhaustiveDependencies: handleOpenSettings는 매 렌더 새로 만들어진다
    useEffect(() => {
        if (!ready) handleOpenSettings(server);
    }, [ready, server.id]);

    if (!ready) {
        return (
            <div className="empty-inline">
                <span className="spinner" aria-hidden="true" />
                <p>{t('shell.loading')}</p>
            </div>
        );
    }
    return (
        <ServerSettingsModal
            {...settingsProps}
            settingsServer={settingsProps.settingsServer as Server}
            section={section}
            modules={modules}
            servers={servers}
        />
    );
}

/** 콘솔 줄은 0.5초마다 붙는다: 상세 전체가 아니라 콘솔만 다시 그린다 */
function LiveServerConsole(props: Omit<ComponentProps<typeof ServerConsole>, 'state'>) {
    const state = useConsoleStore((s) => s.consoles[props.server.id]);
    return <ServerConsole {...props} state={state} />;
}

/** 가동 시간은 1초마다 바뀐다: 상세 전체가 아니라 이 글자만 다시 그린다 */
function Uptime({ startTime }: { startTime: number }) {
    return useServerStore((st) => st.formatUptime(startTime));
}

/** 정보 탭: 숫자·주소·관리 동작 */
function InfoTab({
    server,
    moduleData,
    gameName,
    uptime,
    onDelete,
}: {
    server: Server;
    moduleData?: ModuleInfo;
    gameName: string;
    uptime: ReactNode;
    onDelete: (server: Server) => void;
}) {
    const { t, i18n } = useTranslation('gui');
    const { updateInfo } = useMaintenanceOf(server.id);
    const workingDir = (server.module_settings?.working_dir || server.working_dir || null) as string | null;
    // 빌드 번호와 받은(올라온) 날
    const build = (id: unknown, time: unknown) => {
        const date = buildDate(time, i18n.language);
        return (
            <>
                {t('shell.build', { id })}
                {date && <span className="muted"> ({date})</span>}
            </>
        );
    };
    const rows = [
        [t('shell.info_module'), gameName],
        server.server_build
            ? [t('shell.info_version'), build(server.server_build.id, server.server_build.time)]
            : server.server_version && [t('shell.info_version'), server.server_version],
        updateInfo && [
            t('shell.info_new_version'),
            <span key="new" className="info-new-version">
                {build(updateInfo.remote_buildid, updateInfo.remote_build_time)}
            </span>,
        ],
        uptime && [t('shell.info_uptime'), uptime],
        server.port && [t('shell.info_port'), server.port],
        server.id && [t('shell.info_id'), <code key="id">{server.id}</code>],
        ...dataStatusRows(server),
    ].filter(Boolean) as [string, ReactNode][];
    // 사용자 11-06 #27: 개발 용어(PID·REST·통신 방식)는 '자세히' 안에
    const moreRows = [
        server.status === 'running' && server.pid && ['PID', server.pid],
        server.rest_port && ['REST', `${server.rest_host || '127.0.0.1'}:${server.rest_port}`],
        moduleData && [t('shell.info_protocol'), protocolLabel(server, moduleData)],
    ].filter(Boolean) as [string, ReactNode][];
    const busy = server.status === 'running' || server.status === 'starting' || server.status === 'stopping';

    const facts = (list: [string, ReactNode][]) => (
        <dl className="info-facts">
            {list.map(([k, v]) => (
                <div className="info-fact" key={k}>
                    <dt>{k}</dt>
                    <dd>{v}</dd>
                </div>
            ))}
        </dl>
    );

    return (
        <div className="info-tab ss-panel">
            <Card icon="info" title={t('shell.info_title')}>
                {facts(rows)}
                {workingDir && (
                    <div className="info-folder">
                        <code data-tooltip={workingDir}>{workingDir}</code>
                        <button
                            type="button"
                            className="btn btn-ghost btn-sm"
                            onClick={() => openWithNotice(() => window.api.shellOpenPath(workingDir))}
                        >
                            <Icon name="folder" size="sm" /> {t('shell.open_folder')}
                        </button>
                    </div>
                )}
                {moreRows.length > 0 && <Disclosure summary={t('shell.info_more')}>{facts(moreRows)}</Disclosure>}
            </Card>
            <ServerLogs server={server} />
            <ExtensionSlot slotId="ServerCard.expandedStats" server={server} t={t} />
            <ExtensionSlot slotId="ServerDetail.section" server={server} t={t} />
            <Card icon="tool" title={t('shell.info_check_title')} desc={t('shell.info_check_desc')} className="info-check">
                <ServerCheck server={server} />
                <PortCheck server={server} />
            </Card>
            <Card icon="alertTriangle" title={t('server_settings.danger_zone')} className="ss-danger">
                <SettingRow
                    label={server.linked_folder ? t('server_actions.unlink_confirm_title') : t('shell.delete')}
                    desc={
                        busy
                            ? t('shell.delete_stop_first')
                            : t(server.linked_folder ? 'shell.delete_desc_linked' : 'shell.delete_desc')
                    }
                    control={
                        <button
                            type="button"
                            className="btn btn-danger btn-sm"
                            onClick={() => onDelete(server)}
                            disabled={busy}
                        >
                            <Icon name="trash" size="sm" /> {t('shell.delete')}
                        </button>
                    }
                />
            </Card>
        </div>
    );
}
