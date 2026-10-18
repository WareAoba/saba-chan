import clsx from 'clsx';
import { useTranslation } from 'react-i18next';
import { useInstanceMaintenance, useMaintenanceOf } from '../../hooks/useInstanceMaintenance';
import { useServerStore } from '../../stores/useServerStore';
import { STATUS_TONE, initials, playersText, primaryAction, viewStatus } from '../../utils/serverView';
import { DataBadges } from '../DataSlots';
import ExtensionSlot from '../ExtensionSlot';
import { Icon } from '../Icon';
import { StatusChip } from '../ui/StatusChip';

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
    onContextMenu,
}) {
    const { t } = useTranslation('gui');
    useInstanceMaintenance(server, moduleData);
    const { updateInfo } = useMaintenanceOf(server.id);
    const modulesLoaded = useServerStore((s) => s.modulesLoaded);
    const status = viewStatus(server, moduleData, modulesLoaded);
    const action = primaryAction(status);
    const gameName = t(`mod_${server.module}:module.display_name`, {
        defaultValue: moduleData?.game_name || server.module,
    });
    const statusLabel = t(`shell.status_${status}`);
    const players = playersText(server);

    return (
        <div
            ref={rowRef}
            className={clsx('server-row', { selected, dragging })}
            onPointerDown={(e) => onPointerDown(e, index)}
            onClick={() => {
                if (skipNextClick.current) return;
                onSelect(server.id);
            }}
            onContextMenu={onContextMenu}
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
            <div className="server-row-icon">
                {moduleData?.icon ? <img src={moduleData.icon} alt="" /> : <span>{initials(gameName)}</span>}
                <span className={clsx('server-row-dot', `tone-${STATUS_TONE[status]}`)} aria-hidden="true" />
                <ExtensionSlot slotId="ServerCard.badge" server={server} />
            </div>
            <div className="server-row-text">
                <div className="server-row-name">
                    {server.name}
                    <DataBadges server={server} />
                </div>
                <div className="server-row-sub">
                    <span>{gameName}</span>
                    <span className="sep">·</span>
                    {/* 접속자 수는 실행 중일 때만 있다: 좁은 목록에서는 "실행 중" 대신 접속자 수를 보여 준다 (초록 점·끄기 버튼이 실행 중을 알린다) */}
                    {players ? (
                        <span
                            className="server-row-players"
                            title={`${statusLabel} · ${t('shell.players')} ${players}`}
                            role="img"
                            aria-label={`${statusLabel}, ${t('shell.players')} ${players}`}
                        >
                            <Icon name="user" size={12} />
                            {players}
                        </span>
                    ) : (
                        <StatusChip tone={STATUS_TONE[status]} label={statusLabel} className="bare" />
                    )}
                </div>
            </div>
            {updateInfo && status === 'stopped' && (
                <span className="server-row-flag" title={t('shell.update_available')}>
                    <Icon name="arrowUp" size={14} />
                </span>
            )}
            <button
                type="button"
                className={clsx('icon-btn', 'server-row-action', `action-${action}`)}
                disabled={action === 'busy'}
                onClick={(e) => {
                    e.stopPropagation();
                    if (action === 'stop') onStop(server.name);
                    else if (action === 'start') onStart(server.name, server.module);
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
