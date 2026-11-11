import clsx from 'clsx';
import { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ConsoleEntry } from '../../stores/useConsoleStore';
import type { Server } from '../../stores/useServerStore';
import type { HighlightRule } from '../../types/daemon';
import type { CommandField } from '../../utils/commandRisk';
import { STATUS_TONE } from '../../utils/serverView';
import { Icon } from '../Icon';
import { WindowControls } from '../shell/AppBar';
import { ServerConsole } from './ServerConsole';

interface PopoutConsoleProps {
    popoutParams: { instanceId: string; name: string };
    server?: Server;
    state?: ConsoleEntry;
    highlightRules?: HighlightRule[] | null;
    quickCommands?: CommandField[];
    openConsole: (id: string, name: string) => void;
    setConsoleInput: (id: string, value: string) => void;
    sendConsoleCommand: (id: string) => void;
    readOnly?: boolean;
}

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
    readOnly,
}: PopoutConsoleProps) {
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

    // 사용자가 닫으면 다음에 다시 띄우지 않는다 (서버가 꺼져 닫힐 때·앱을 끌 때는 기억한다)
    const handleClose = useCallback(() => {
        const close = window.api?.consolePopoutClose;
        if (typeof close !== 'function') return window.api?.closeWindow();
        Promise.resolve(close(popoutParams.instanceId, false)).catch(() => window.api?.closeWindow());
    }, [popoutParams.instanceId]);

    const name = server?.name || popoutParams.name;
    const status = server?.status || 'stopped';

    return (
        <div className="App console-popout-app">
            <header className="appbar console-popout-titlebar" data-tauri-drag-region>
                <div className="pip-pill pip-title" data-tauri-drag-region>
                    <span
                        className={clsx('pip-dot', `tone-${STATUS_TONE[status] ?? 'idle'}`, {
                            pulse: status === 'starting' || status === 'stopping',
                        })}
                        data-status={status}
                        aria-hidden="true"
                        data-tauri-drag-region
                    />
                    <span className="pip-name" data-tauri-drag-region>
                        {name}
                    </span>
                    <span className="pip-kind" data-tauri-drag-region>
                        <Icon name="terminal" size={12} />
                        {t('console.popout_kind')}
                    </span>
                </div>
                <div className="appbar-spacer" data-tauri-drag-region />
                <div className="pip-tools">
                    <button
                        type="button"
                        className="pip-tool"
                        onClick={handlePopin}
                        data-tooltip={t('console.popin', { name })}
                        aria-label={t('console.popin', { name })}
                    >
                        <Icon name="externalLinkIn" size={14} />
                    </button>
                    <button
                        type="button"
                        className={`pip-tool${pinned ? ' active' : ''}`}
                        onClick={handleTogglePin}
                        data-tooltip={pinned ? t('console.unpin') : t('console.pin')}
                        aria-label={pinned ? t('console.unpin') : t('console.pin')}
                        aria-pressed={pinned}
                    >
                        <Icon name="pin" size={14} />
                    </button>
                </div>
                <WindowControls onClose={handleClose} />
            </header>
            <div className="console-popout-body">
                <ServerConsole
                    server={server || ({ id: popoutParams.instanceId, name: popoutParams.name } as Server)}
                    state={state}
                    highlightRules={highlightRules}
                    quickCommands={quickCommands}
                    openConsole={openConsole}
                    setConsoleInput={setConsoleInput}
                    sendConsoleCommand={sendConsoleCommand}
                    readOnly={readOnly}
                    autoFocus
                />
            </div>
        </div>
    );
}
