import { useTranslation } from 'react-i18next';
import { Icon } from './index';

/**
 * ConsoleDock — A dock/taskbar at the bottom of the app showing icons for each
 * open console instance. Clicking an icon restores a minimized console or focuses it.
 */
export function ConsoleDock({
    consoles,
    restoreConsole,
    focusConsole,
    closeConsole,
    popinConsole,
    consolePopoutInstanceId,
    servers,
    hasProgressBar,
}) {
    const { t } = useTranslation('gui');
    const entries = Object.entries(consoles);
    // Also show a dock item for the popout instance if one exists
    const hasPopout = !!consolePopoutInstanceId;
    const popoutServer = hasPopout ? servers?.find((s) => s.id === consolePopoutInstanceId) : null;

    if (entries.length === 0 && !hasPopout) return null;

    return (
        <div className={`console-dock${hasProgressBar ? ' console-dock-above-progress' : ''}`}>
            <div className="console-dock-inner">
                {entries.map(([instanceId, state]) => (
                    <button
                        key={instanceId}
                        className={`console-dock-item ${state.minimized ? 'console-dock-minimized' : 'console-dock-active'}${state.pinned ? ' console-dock-pinned' : ''}`}
                        onClick={() => {
                            if (state.minimized) {
                                restoreConsole(instanceId);
                            } else {
                                focusConsole(instanceId);
                            }
                        }}
                        onAuxClick={(e) => {
                            // Middle-click to close
                            if (e.button === 1) {
                                e.preventDefault();
                                closeConsole(instanceId);
                            }
                        }}
                        title={
                            state.minimized
                                ? t('console.dock_restore', { name: state.server.name, defaultValue: `Restore: ${state.server.name}` })
                                : t('console.dock_focus', { name: state.server.name, defaultValue: `Focus: ${state.server.name}` })
                        }
                    >
                        <span className="console-dock-icon">
                            <Icon name="terminal" size="sm" />
                        </span>
                        <span className="console-dock-label">{state.server.name}</span>
                        {state.pinned && <span className="console-dock-pin-indicator"><Icon name="pin" size="xs" /></span>}
                        {state.minimized && <span className="console-dock-minimized-indicator" />}
                    </button>
                ))}
                {/* Popout instance — clicking brings it back */}
                {hasPopout && popoutServer && (
                    <button
                        key="popout"
                        className="console-dock-item console-dock-popout"
                        onClick={() => popinConsole(consolePopoutInstanceId)}
                        title={t('console.popin', { name: popoutServer.name, defaultValue: `Bring back: ${popoutServer.name}` })}
                    >
                        <span className="console-dock-icon">
                            <Icon name="external-link-in" size="sm" />
                        </span>
                        <span className="console-dock-label">{popoutServer.name}</span>
                        <span className="console-dock-popout-badge">PiP</span>
                    </button>
                )}
            </div>
        </div>
    );
}
