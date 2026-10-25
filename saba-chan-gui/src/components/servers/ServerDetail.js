import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
    applyInstanceUpdate,
    closeCookedCard,
    dismissInstanceProvision,
    useMaintenanceOf,
} from '../../hooks/useInstanceMaintenance';
import { useConsoleStore } from '../../stores/useConsoleStore';
import { useServerStore } from '../../stores/useServerStore';
import { useSettingsStore } from '../../stores/useSettingsStore';
import { failedModuleReason, failureForModule } from '../../utils/failedModules';
import { openWithNotice, safeShowToast } from '../../utils/helpers';
import { useUIStore } from '../../stores/useUIStore';
import {
    STATUS_TONE,
    consoleQuickCommands,
    gameNameOf,
    initials,
    interactionOf,
    playersText,
    primaryAction,
    protocolLabel,
    viewStatus,
} from '../../utils/serverView';
import { dataStatusRows } from '../DataSlots';
import ExtensionSlot from '../ExtensionSlot';
import { Icon } from '../Icon';
import { ServerCheck } from './ServerCheck';
import CommandModal from '../Modals/CommandModal';
import { NativeProvision } from '../NativeProvision';
import { ServerSettingsModal } from '../ServerSettingsModal';
import { Segmented, StatusChip } from '../ui/StatusChip';
import { ExitHistory } from './ExitHistory';
import { InstanceDiscordTab, instanceWordsAvailable } from '../discord/InstanceWordsTab';
import { AuditLog, auditAvailable } from './AuditTab';
import { MaintenanceTab, maintenanceAvailable } from './MaintenanceTab';
import { ModuleAuditNotice } from './ModuleAuditNotice';
import { OpenInTerminalButton } from './OpenInTerminal';
import { ServerConsole } from './ServerConsole';

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
    setModal,
    consoleProps,
    settingsProps,
}) {
    const { t } = useTranslation('gui');
    const portConflictCheck = useSettingsStore((s) => s.portConflictCheck);
    const { provisionProgress, cooked, updateInfo, updatingServer } = useMaintenanceOf(server.id);
    const modulesLoaded = useServerStore((s) => s.modulesLoaded);
    const status = viewStatus(server, moduleData, modulesLoaded);
    const moduleFailure = useServerStore((st) =>
        status === 'missing' ? failureForModule(st.failedModules, server.module) : null,
    );
    const action = primaryAction(status);
    const interaction = interactionOf(server, moduleData);
    const gameName = gameNameOf(t, server.module, moduleData);

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
        { id: 'settings', label: t('shell.tab_settings'), icon: <Icon name="settings" size="sm" /> },
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
        auditAvailable() && { id: 'audit', label: t('shell.tab_audit'), icon: <Icon name="file" size="sm" /> },
        { id: 'info', label: t('shell.tab_info'), icon: <Icon name="info" size="sm" /> },
    ].filter(Boolean);

    // null = 아직 고르지 않음 → 아래 defaultTab
    const [tab, setTab] = useState(null);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 서버가 바뀔 때만 초기화
    useEffect(() => {
        setTab(null);
    }, [server.id]);
    // 우클릭 메뉴 등 바깥에서 특정 탭을 요청하면 그 탭으로
    const requestedTab = useUIStore((s) => s.requestedServerTab);
    useEffect(() => {
        if (!requestedTab) return;
        setTab(requestedTab);
        useUIStore.getState().clearRequestedServerTab();
    }, [requestedTab]);
    // 기본 탭: 입력을 받는 콘솔이면 콘솔, 켜져 있으면 명령, 꺼져 있으면 정보 (쓸 수 있는 것부터)
    const defaultTab = interaction.input
        ? 'console'
        : server.status === 'running' && interaction.commands
          ? 'commands'
          : 'info';
    const activeTab = tabs.some((x) => x.id === tab) ? tab : defaultTab;

    const conflicts = portConflictCheck ? server.port_conflicts || [] : [];
    const uptime = status === 'running' && server.start_time ? <Uptime startTime={server.start_time} /> : null;
    const players = playersText(server);

    const primaryButton = (() => {
        if (status === 'missing') return null;
        if (updateInfo && status === 'stopped') {
            return (
                <button
                    type="button"
                    className="btn btn-info"
                    disabled={updatingServer}
                    onClick={() => applyInstanceUpdate(server, setServers)}
                    title={`${updateInfo.local_buildid} → ${updateInfo.remote_buildid}`}
                >
                    <Icon name="arrowUp" size="sm" /> {updatingServer ? t('shell.updating') : t('shell.update')}
                </button>
            );
        }
        if (action === 'busy') {
            return (
                <button type="button" className="btn btn-ghost btn-lg" disabled>
                    <span className="spinner" aria-hidden="true" /> {t(`shell.status_${status}`)}
                </button>
            );
        }
        return action === 'stop' ? (
            <>
                {onRestart && window.api?.instanceRestart && (
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
            <button
                type="button"
                className="btn btn-primary btn-lg"
                onClick={() => onStart(server.name, server.module)}
            >
                <Icon name="play" size="sm" /> {t('shell.start')}
            </button>
        );
    })();

    return (
        <section className="server-detail" aria-label={server.name}>
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
                        {server.server_version && <span className="tag">{server.server_version}</span>}
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
                        pulse={status === 'starting' || status === 'stopping'}
                    />
                    {status !== 'missing' && <OpenInTerminalButton server={server} />}
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
                <div className="notice tone-warn" role="status">
                    <Icon name="alertTriangle" size="sm" />
                    <span>{t('server_status.unresponsive', { reason: server.unresponsive })}</span>
                </div>
            )}

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

            {(server.provisioning || cooked) && (
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
                        provisionProgress={provisionProgress}
                        cooked={cooked}
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
                    <Segmented items={tabs} value={activeTab} onChange={setTab} ariaLabel={server.name} />
                    <div className="detail-body" key={`${server.id}-${activeTab}`}>
                        {activeTab === 'console' && (
                            <LiveServerConsole
                                server={server}
                                readOnly={!interaction.input}
                                highlightRules={moduleData?.syntax_highlight?.rules || null}
                                quickCommands={consoleQuickCommands(moduleData?.commands?.fields, moduleData?.command_aliases)}
                                openConsole={consoleProps.openConsole}
                                setConsoleInput={consoleProps.setConsoleInput}
                                sendConsoleCommand={consoleProps.sendConsoleCommand}
                                poppedOut={consoleProps.consolePopoutIds.includes(server.id)}
                                onPopout={async () => {
                                    // 창이 열리면 셸이 console:popoutOpened로 알려 준다. 못 열면 알린다 (G-C5)
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
                                }}
                                onPopin={() => consoleProps.popinConsole(server.id)}
                            />
                        )}
                        {activeTab === 'commands' &&
                            (server.status === 'running' ? (
                                <CommandModal
                                    server={server}
                                    modules={modules}
                                    onClose={() => {}}
                                    onExecute={setModal}
                                    embedded
                                />
                            ) : (
                                <div className="empty-inline">
                                    <Icon name="command" size="lg" />
                                    <p>{t('shell.commands_offline')}</p>
                                </div>
                            ))}
                        {activeTab === 'settings' && (
                            <SettingsTab
                                server={server}
                                settingsProps={settingsProps}
                                modules={modules}
                                servers={servers}
                            />
                        )}
                        {activeTab === 'maintenance' && <MaintenanceTab server={server} />}
                        {activeTab === 'discord' && <InstanceDiscordTab server={server} />}
                        {activeTab === 'audit' && <AuditLog instanceId={server.id} />}
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
function ServerName({ server, canRename, onRename }) {
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
                {canRename && window.api?.instanceRename && (
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
        const ok = await onRename(server.id, next);
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
                if (e.key === 'Enter') e.currentTarget.blur();
                if (e.key === 'Escape') {
                    setValue(server.name);
                    setEditing(false);
                }
            }}
        />
    );
}

/** 설정 탭: 예전 "서버 설정" 모달 내용을 그대로 박아 넣는다 */
function SettingsTab({ server, settingsProps, modules, servers }) {
    const { t } = useTranslation('gui');
    const { showSettingsModal, settingsServer, handleOpenSettings } = settingsProps;
    const ready = showSettingsModal && settingsServer?.id === server.id;

    // 탭을 열거나 저장 뒤 닫히면 최신 값으로 다시 연다
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
    return <ServerSettingsModal {...settingsProps} modules={modules} servers={servers} embedded onClose={() => {}} />;
}

/** 콘솔 줄은 0.5초마다 붙는다: 상세 전체가 아니라 콘솔만 다시 그린다 */
function LiveServerConsole(props) {
    const state = useConsoleStore((s) => s.consoles[props.server.id]);
    return <ServerConsole {...props} state={state} />;
}

/** 가동 시간은 1초마다 바뀐다: 상세 전체가 아니라 이 글자만 다시 그린다 */
function Uptime({ startTime }) {
    return useServerStore((st) => st.formatUptime(startTime));
}

/** 정보 탭: 숫자·주소·관리 동작 */
function InfoTab({ server, moduleData, gameName, uptime, onDelete }) {
    const { t } = useTranslation('gui');
    const supported = moduleData?.protocols?.supported || [];
    const workingDir = server.module_settings?.working_dir || server.working_dir || null;
    const rows = [
        [t('shell.info_module'), gameName],
        server.server_version && [t('shell.info_version'), server.server_version],
        uptime && [t('shell.info_uptime'), uptime],
        server.status === 'running' && server.pid && ['PID', server.pid],
        server.port && [t('shell.info_port'), server.port],
        server.rest_port && ['REST', `${server.rest_host || '127.0.0.1'}:${server.rest_port}`],
        moduleData && [t('shell.info_protocol'), protocolLabel(server, moduleData)],
        server.id && [t('shell.info_id'), <code key="id">{server.id}</code>],
        ...dataStatusRows(server),
    ].filter(Boolean);
    const busy = server.status === 'running' || server.status === 'starting' || server.status === 'stopping';

    return (
        <div className="info-tab">
            <dl className="info-grid">
                {rows.map(([k, v]) => (
                    <div className="info-cell" key={k}>
                        <dt>{k}</dt>
                        <dd>{v}</dd>
                    </div>
                ))}
            </dl>
            <ExitHistory server={server} />
            <ExtensionSlot slotId="ServerCard.expandedStats" server={server} t={t} />
            <ExtensionSlot slotId="ServerDetail.section" server={server} t={t} />
            <ServerCheck server={server} />
            <div className="info-actions">
                {workingDir && (
                    <button
                        type="button"
                        className="btn btn-ghost"
                        onClick={() => openWithNotice(() => window.api.shellOpenPath(workingDir))}
                    >
                        <Icon name="folder" size="sm" /> {t('shell.open_folder')}
                    </button>
                )}
                <button
                    type="button"
                    className="btn btn-ghost btn-danger-text"
                    onClick={() => onDelete(server)}
                    disabled={busy}
                    title={busy ? t('shell.delete_stop_first') : undefined}
                >
                    <Icon name="trash" size="sm" /> {t('shell.delete')}
                </button>
            </div>
        </div>
    );
}
