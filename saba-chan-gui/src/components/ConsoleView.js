import clsx from 'clsx';
import { useCallback, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSettingsStore } from '../stores/useSettingsStore';
import { getCachedRules } from '../utils/syntaxHighlight';
import { Icon } from './index';
import { HighlightedContent } from './HighlightedContent';

/**
 * ConsolePanel — embedded console panel for managed server processes.
 */
export function ConsolePanel({
    consoleServer,
    consoleLines,
    consoleInput,
    setConsoleInput,
    consoleEndRef,
    sendConsoleCommand,
    closeConsole,
    setConsolePopoutInstanceId,
    highlightRules,
    stdinDisabled,
}) {
    const { t } = useTranslation('gui');
    const syntaxEnabled = useSettingsStore((s) => s.consoleSyntaxHighlight);

    // 하이라이팅 규칙 컴파일 (모듈 이름 기준 캐싱)
    const compiledRules = useMemo(
        () => (syntaxEnabled && highlightRules ? getCachedRules(consoleServer?.name || '_', highlightRules) : []),
        [highlightRules, consoleServer?.name, syntaxEnabled],
    );

    return (
        <div className="console-panel">
            <div className="console-header">
                <span className="console-title">
                    <span className="console-icon">{'>'}_</span>
                    {consoleServer.name}
                </span>
                <div className="console-header-actions">
                    <button type="button"
                        className="console-popout-btn"
                        aria-label={t('console.popout')}
                        onClick={async () => {
                            try {
                                const serverId = consoleServer.id;
                                const serverName = consoleServer.name;
                                const result = await window.api.consolePopout(serverId, serverName);
                                if (result?.ok) setConsolePopoutInstanceId(serverId);
                                closeConsole();
                            } catch (err) {
                                console.error('Popout failed:', err);
                            }
                        }}
                        data-tooltip={t('console.popout', { defaultValue: 'Pop out to separate window' })}
                    >
                        <Icon name="external-link" size="sm" />
                    </button>
                    <button type="button" className="console-close" onClick={closeConsole} aria-label={t('modals.close')} data-tooltip={t('modals.close')}>
                        &times;
                    </button>
                </div>
            </div>
            <div className="console-output">
                {consoleLines.length === 0 && <div className="console-empty">{t('console.waiting')}</div>}
                {consoleLines.map((line) => (
                    <div
                        key={line.id}
                        className={clsx(
                            'console-line',
                            `console-${line.source?.toLowerCase() || 'stdout'}`,
                            `console-level-${line.level?.toLowerCase() || 'info'}`,
                        )}
                    >
                        <span className="console-content">
                            <HighlightedContent text={line.content} compiledRules={compiledRules} />
                        </span>
                    </div>
                ))}
                <div ref={consoleEndRef} />
            </div>
            <div className="console-input-row">
                <span className="console-prompt">{'>'}</span>
                <div className={clsx('console-input-wrap', stdinDisabled && 'console-input-disabled')}>
                    {stdinDisabled && <Icon name="alertTriangle" size="sm" />}
                    <input
                        type="text"
                        className="console-input"
                        value={consoleInput}
                        onChange={(e) => setConsoleInput(e.target.value)}
                        onKeyDown={(e) => {
                            if (e.key === 'Enter') sendConsoleCommand();
                        }}
                        placeholder={stdinDisabled ? t('console.stdin_disabled') : t('console.input_placeholder')}
                        disabled={stdinDisabled}
                    />
                </div>
                <button type="button" className="console-send" onClick={sendConsoleCommand} disabled={stdinDisabled}>
                    {t('console.send')}
                </button>
            </div>
        </div>
    );
}

/**
 * PopoutConsole — full-window console for popout mode.
 */
export function PopoutConsole({
    popoutParams,
    consoleLines,
    consoleInput,
    setConsoleInput,
    consoleEndRef,
    sendConsoleCommand,
    highlightRules,
    stdinDisabled,
}) {
    const { t } = useTranslation('gui');
    const [pinned, setPinned] = useState(true); // popout defaults to always-on-top
    const syntaxEnabled = useSettingsStore((s) => s.consoleSyntaxHighlight);

    const compiledRules = useMemo(
        () => (syntaxEnabled && highlightRules ? getCachedRules(popoutParams?.name || '_', highlightRules) : []),
        [highlightRules, popoutParams?.name, syntaxEnabled],
    );

    const handlePopin = useCallback(async () => {
        try {
            await window.api?.consolePopin(popoutParams?.instanceId);
        } catch (err) {
            console.error('Popin failed:', err);
        }
    }, [popoutParams?.instanceId]);

    const handleTogglePin = useCallback(async () => {
        const newPinned = !pinned;
        setPinned(newPinned);
        try {
            await window.api?.consoleToggleAlwaysOnTop(popoutParams?.instanceId, newPinned);
        } catch (err) {
            console.error('Toggle pin failed:', err);
            setPinned(pinned); // revert on error
        }
    }, [pinned, popoutParams?.instanceId]);

    return (
        <div className="App console-popout-app">
            <div className="console-popout-titlebar">
                <span className="console-popout-titlebar-title">
                    <span className="console-icon">{'>'}_</span>
                    {popoutParams.name}
                </span>
                <div className="console-popout-titlebar-controls">
                    <button type="button"
                        className={`console-popout-titlebar-btn console-popout-titlebar-popin`}
                        aria-label={t('console.popin')}
                        onClick={handlePopin}
                        data-tooltip={t('console.popin', { defaultValue: 'Bring back to main window' })}
                    >
                        <Icon name="external-link-in" size="sm" />
                    </button>
                    <button type="button"
                        className={`console-popout-titlebar-btn console-popout-titlebar-pin${pinned ? ' console-popout-titlebar-pinned' : ''}`}
                        aria-label={pinned ? t('console.unpin') : t('console.pin')}
                        onClick={handleTogglePin}
                        data-tooltip={pinned ? t('console.unpin', { defaultValue: 'Unpin' }) : t('console.pin', { defaultValue: 'Pin on top' })}
                    >
                        <Icon name="pin" size="sm" />
                    </button>
                    <button type="button"
                        className="console-popout-titlebar-btn"
                        aria-label={t('title_bar.minimize')}
                        onClick={() => window.electron?.minimizeWindow()}
                        data-tooltip={t('title_bar.minimize')}
                    >
                        ─
                    </button>
                    <button type="button"
                        className="console-popout-titlebar-btn console-popout-titlebar-close"
                        aria-label={t('title_bar.close')}
                        onClick={() => window.electron?.closeWindow()}
                        data-tooltip={t('title_bar.close')}
                    >
                        &times;
                    </button>
                </div>
            </div>
            <div className="console-popout-body">
                <div className="console-output">
                    {consoleLines.length === 0 && <div className="console-empty">{t('console.waiting')}</div>}
                    {consoleLines.map((line) => (
                        <div
                            key={line.id}
                            className={clsx(
                                'console-line',
                                `console-${line.source?.toLowerCase() || 'stdout'}`,
                                `console-level-${line.level?.toLowerCase() || 'info'}`,
                            )}
                        >
                            <span className="console-content">
                                <HighlightedContent text={line.content} compiledRules={compiledRules} />
                            </span>
                        </div>
                    ))}
                    <div ref={consoleEndRef} />
                </div>
                <div className="console-input-row">
                    <span className="console-prompt">{'>'}</span>
                    <div className={clsx('console-input-wrap', stdinDisabled && 'console-input-disabled')}>
                        {stdinDisabled && <Icon name="alertTriangle" size="sm" />}
                        <input
                            type="text"
                            className="console-input"
                            value={consoleInput}
                            onChange={(e) => setConsoleInput(e.target.value)}
                            onKeyDown={(e) => {
                                if (e.key === 'Enter') sendConsoleCommand();
                            }}
                            placeholder={stdinDisabled ? t('console.stdin_disabled') : t('console.input_placeholder')}
                                disabled={stdinDisabled}
                        />
                    </div>
                    <button type="button" className="console-send" onClick={sendConsoleCommand} disabled={stdinDisabled}>
                        {t('console.send')}
                    </button>
                </div>
            </div>
        </div>
    );
}
