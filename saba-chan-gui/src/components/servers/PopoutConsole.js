import { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { ServerConsole } from './ServerConsole';

/**
 * 따로 띄운 콘솔 창(PiP). 제목줄만 이 창의 것이고, 본문은 상세 화면 콘솔을 그대로 쓴다
 * (명령 기록·위험 명령 확인·빠른 명령·자동 스크롤이 같다).
 */
export function PopoutConsole({
    popoutParams,
    server,
    state,
    highlightRules,
    quickCommands,
    openConsole,
    setConsoleInput,
    sendConsoleCommand,
}) {
    const { t } = useTranslation('gui');
    const [pinned, setPinned] = useState(true); // 처음엔 항상 위 (셸이 항상 위로 연다)

    const handlePopin = useCallback(async () => {
        try {
            await window.api?.consolePopin(popoutParams.instanceId);
        } catch (err) {
            console.error('Popin failed:', err);
        }
    }, [popoutParams.instanceId]);

    const handleTogglePin = useCallback(async () => {
        const newPinned = !pinned;
        setPinned(newPinned);
        try {
            await window.api?.consoleToggleAlwaysOnTop(popoutParams.instanceId, newPinned);
        } catch (err) {
            console.error('Toggle pin failed:', err);
            setPinned(pinned);
        }
    }, [pinned, popoutParams.instanceId]);

    return (
        <div className="App console-popout-app">
            <div className="console-popout-titlebar">
                <span className="console-popout-titlebar-title">
                    <Icon name="terminal" size="sm" />
                    {server?.name || popoutParams.name}
                </span>
                <div className="console-popout-titlebar-controls">
                    <button
                        type="button"
                        className="console-popout-titlebar-btn"
                        onClick={handlePopin}
                        title={t('console.popin')}
                        aria-label={t('console.popin')}
                    >
                        <Icon name="externalLinkIn" size="sm" />
                    </button>
                    <button
                        type="button"
                        className={`console-popout-titlebar-btn${pinned ? ' console-popout-titlebar-pinned' : ''}`}
                        onClick={handleTogglePin}
                        title={pinned ? t('console.unpin') : t('console.pin')}
                        aria-label={pinned ? t('console.unpin') : t('console.pin')}
                        aria-pressed={pinned}
                    >
                        <Icon name="pin" size="sm" />
                    </button>
                    <button
                        type="button"
                        className="console-popout-titlebar-btn"
                        onClick={() => window.electron?.minimizeWindow()}
                        title={t('title_bar.minimize')}
                        aria-label={t('title_bar.minimize')}
                    >
                        <Icon name="minus" size="sm" />
                    </button>
                    <button
                        type="button"
                        className="console-popout-titlebar-btn console-popout-titlebar-close"
                        onClick={() => window.electron?.closeWindow()}
                        title={t('title_bar.close')}
                        aria-label={t('title_bar.close')}
                    >
                        <Icon name="close" size="sm" />
                    </button>
                </div>
            </div>
            <div className="console-popout-body">
                <ServerConsole
                    server={server || { id: popoutParams.instanceId, name: popoutParams.name }}
                    state={state}
                    highlightRules={highlightRules}
                    quickCommands={quickCommands}
                    openConsole={openConsole}
                    setConsoleInput={setConsoleInput}
                    sendConsoleCommand={sendConsoleCommand}
                    autoFocus
                />
            </div>
        </div>
    );
}
