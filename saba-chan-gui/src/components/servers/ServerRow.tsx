import clsx from 'clsx';
import type { MutableRefObject, PointerEvent } from 'react';
import { useTranslation } from 'react-i18next';
import { useInstanceMaintenance, useMaintenanceOf } from '../../hooks/useInstanceMaintenance';
import { type Server, useServerStore } from '../../stores/useServerStore';
import { useUIStore } from '../../stores/useUIStore';
import type { ModuleInfo } from '../../types/daemon';
import { openWithNotice } from '../../utils/helpers';
import {
    STATUS_TONE,
    isTransitional,
    gameNameOf,
    initials,
    interactionOf,
    playersText,
    primaryAction,
    viewStatus,
} from '../../utils/serverView';
import { DataBadges } from '../DataSlots';
import ExtensionSlot from '../ExtensionSlot';
import { Icon } from '../Icon';
import { type ContextMenuItem, useContextMenu } from '../ui/ContextMenu';
import { StatusChip } from '../ui/StatusChip';

interface ServerRowProps {
    server: Server;
    index: number;
    moduleData?: ModuleInfo;
    selected: boolean;
    onSelect: (id: string) => void;
    onStart: (id: string) => void;
    onStop: (id: string) => void;
    rowRef: (el: HTMLDivElement | null) => void;
    dragging: boolean;
    onPointerDown: (e: PointerEvent<HTMLDivElement>, index: number) => void;
    skipNextClick: MutableRefObject<boolean>;
    onRestart?: (id: string) => void;
    onDelete?: (server: Server) => void;
    /** 브랜치 서버 줄: 상위 서버 아래에 가지 선으로 붙는다 */
    branch?: boolean;
    /** 이 서버 아래 브랜치 서버 수 (0이면 접기 단추가 없다) */
    branches?: number;
    branchesCollapsed?: boolean;
    onToggleBranches?: (id: string) => void;
    /** 이 서버의 브랜치 서버 만들기 (브랜치에는 주지 않는다: 한 단계만 둔다) */
    onBranch?: (server: Server) => void;
    /** 저장하지 않은 설정이 메모리에 남아 있다 (useServerSettings `unsavedServers`) */
    unsaved?: boolean;
}

/**
 * 서버 목록의 한 줄. 상태를 글자로 말하고, 켜기/끄기 버튼은 항상 같은 자리에 있다 (L 원칙 2·3).
 */
export function ServerRow({
    server,
    index,
    moduleData,
    selected,
    onSelect,
    onStart,
    onStop,
    rowRef,
    dragging,
    onPointerDown,
    skipNextClick,
    onRestart,
    onDelete,
    branch,
    branches = 0,
    branchesCollapsed = false,
    onToggleBranches,
    onBranch,
    unsaved,
}: ServerRowProps) {
    const { t } = useTranslation('gui');
    useInstanceMaintenance(server, moduleData);
    const { updateInfo, updateCheckError } = useMaintenanceOf(server.id);
    const modulesLoaded = useServerStore((s) => s.modulesLoaded);
    const status = viewStatus(server, moduleData, modulesLoaded);
    const action = primaryAction(status);
    const gameName = gameNameOf(t, server.module, moduleData);
    const statusLabel = t(`shell.status_${status}`);
    const players = playersText(server);
    const openMenu = useContextMenu();
    // 좁은 창에서는 목록만 보이므로 상세도 드러낸다 (바깥에서 서버를 여는 길과 같다)
    const showTab = (tab: string) => {
        const ui = useUIStore.getState();
        ui.showServerTab(server.id, tab);
        ui.revealServer(server.id);
    };
    const dir = (server.module_settings?.working_dir || server.working_dir || null) as string | null;
    const menuItems = (): ContextMenuItem[] => [
        action === 'stop' && { id: 'stop', label: t('shell.stop'), icon: 'stop', onSelect: () => onStop(server.id) },
        action === 'start' && {
            id: 'start',
            label: t('shell.start'),
            icon: 'play',
            onSelect: () => onStart(server.id),
        },
        status === 'running' &&
            onRestart &&
            typeof window.api?.instanceRestart === 'function' && {
                id: 'restart',
                label: t('shell.restart'),
                icon: 'refresh',
                onSelect: () => onRestart(server.id),
            },
        interactionOf(server, moduleData).console && {
            id: 'console',
            label: t('context_menu.open_console'),
            icon: 'terminal',
            onSelect: () => showTab('console'),
        },
        'separator',
        onBranch && { id: 'branch', label: t('branch.create'), icon: 'branch', onSelect: () => onBranch(server) },
        { id: 'settings', label: t('context_menu.settings'), icon: 'settings', onSelect: () => showTab('settings') },
        !!dir && {
            id: 'folder',
            label: t('shell.open_folder'),
            icon: 'folder',
            onSelect: () => openWithNotice(() => window.api.shellOpenPath(dir)),
        },
        onDelete && 'separator',
        onDelete && {
            id: 'delete',
            label: t('context_menu.delete'),
            icon: 'trash',
            danger: true,
            onSelect: () => onDelete(server),
        },
    ];

    return (
        <div
            ref={rowRef}
            className={clsx('server-row', { selected, dragging, branch })}
            onPointerDown={(e) => onPointerDown(e, index)}
            onClick={() => {
                if (skipNextClick.current) return;
                onSelect(server.id);
            }}
            onContextMenu={(e) => openMenu(e, menuItems())}
            onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    onSelect(server.id);
                }
            }}
            role="option"
            aria-selected={selected}
            tabIndex={0}
            data-status={status}
        >
            {branches > 0 && (
                <button
                    type="button"
                    className="icon-btn server-row-fold"
                    aria-expanded={!branchesCollapsed}
                    aria-label={t(branchesCollapsed ? 'branch.expand' : 'branch.collapse', { name: server.name })}
                    onClick={(e) => {
                        e.stopPropagation();
                        onToggleBranches?.(server.id);
                    }}
                >
                    <Icon name={branchesCollapsed ? 'chevronRight' : 'chevronDown'} size={14} />
                </button>
            )}
            <div className="server-row-icon">
                {moduleData?.icon ? <img src={moduleData.icon} alt="" /> : <span>{initials(gameName)}</span>}
                <span
                    className={clsx('server-row-dot', `tone-${STATUS_TONE[status]}`, { pulse: isTransitional(status) })}
                    aria-hidden="true"
                />
                <ExtensionSlot slotId="ServerCard.badge" server={server} />
            </div>
            <div className="server-row-text">
                <div className="server-row-name">
                    {branches > 0 && branchesCollapsed ? (
                        // 접어 둔 상위 서버: 이름이 길어도 브랜치 수는 잘리지 않는다 (좁은 목록이라 아이콘과 수만 보인다)
                        <>
                            <span className="server-row-name-text">{server.name}</span>
                            <span
                                className="tag server-row-branches"
                                role="img"
                                aria-label={t('branch.count', { count: branches })}
                                data-tooltip={t('branch.count', { count: branches })}
                            >
                                <Icon name="branch" size={11} />
                                {branches}
                            </span>
                        </>
                    ) : (
                        server.name
                    )}
                    <DataBadges server={server} />
                </div>
                <div className="server-row-sub">
                    <span>{gameName}</span>
                    <span className="sep">·</span>
                    {/* 접속자 수는 실행 중일 때만 있다: 좁은 목록에서는 "실행 중" 대신 접속자 수를 보여 준다 (초록 점·끄기 버튼이 실행 중을 알린다) */}
                    {players ? (
                        <span
                            className="server-row-players"
                            data-tooltip={`${statusLabel} · ${t('shell.players')} ${players}`}
                            role="img"
                            aria-label={`${statusLabel}, ${t('shell.players')} ${players}`}
                        >
                            <Icon name="user" size={12} />
                            {players}
                        </span>
                    ) : (
                        <StatusChip
                            tone={STATUS_TONE[status]}
                            pulse={isTransitional(status)}
                            label={statusLabel}
                            className="bare"
                        />
                    )}
                </div>
            </div>
            {unsaved && (
                <span
                    className="server-row-flag tone-warn"
                    data-tooltip={t('server_settings.unsaved_flag')}
                    role="img"
                    aria-label={t('server_settings.unsaved_flag')}
                >
                    <Icon name="alertTriangle" size={14} />
                </span>
            )}
            {/* 켜져 있지만 상태 질문에 계속 답하지 않는다 (데몬 unresponsive: 모듈의 마지막 실패 이유) */}
            {server.unresponsive && status === 'running' && (
                <span
                    className="server-row-flag tone-warn"
                    data-tooltip={t('server_status.unresponsive', { reason: server.unresponsive })}
                    role="img"
                    aria-label={t('server_status.unresponsive', { reason: server.unresponsive })}
                >
                    <Icon name="alertTriangle" size={14} />
                </span>
            )}
            {/* 게임 서버 업데이트를 확인하지 못했다 (SteamCMD 실패 등): "최신"으로 오해하지 않게 */}
            {updateCheckError != null && (
                <span
                    className="server-row-flag tone-warn"
                    data-tooltip={t('shell.update_check_failed', { reason: updateCheckError })}
                    role="img"
                    aria-label={t('shell.update_check_failed', { reason: updateCheckError })}
                >
                    <Icon name="alertTriangle" size={14} />
                </span>
            )}
            {updateInfo && status === 'stopped' && (
                <span
                    className="server-row-flag"
                    data-tooltip={t('shell.update_available')}
                    role="img"
                    aria-label={t('shell.update_available')}
                >
                    <Icon name="arrowUp" size={14} />
                </span>
            )}
            <button
                type="button"
                className={clsx('icon-btn', 'server-row-action', `action-${action}`)}
                disabled={action === 'busy'}
                onClick={(e) => {
                    e.stopPropagation();
                    if (action === 'stop') onStop(server.id);
                    else if (action === 'start') onStart(server.id);
                }}
                data-tooltip={action === 'stop' ? t('shell.stop') : action === 'start' ? t('shell.start') : statusLabel}
                aria-label={`${server.name} ${action === 'stop' ? t('shell.stop') : t('shell.start')}`}
            >
                {action === 'busy' ? (
                    <span className="spinner" aria-hidden="true" />
                ) : (
                    <Icon name={action === 'stop' ? 'stop' : 'play'} size="sm" />
                )}
            </button>
        </div>
    );
}
