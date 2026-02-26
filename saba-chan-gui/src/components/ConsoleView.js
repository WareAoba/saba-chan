import clsx from 'clsx';
import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { getCachedRules, highlightLine } from '../utils/syntaxHighlight';
import { Icon } from './index';

/**
 * 단일 콘솔 라인의 content를 하이라이팅하여 렌더링
 */
function HighlightedContent({ text, compiledRules }) {
    const segments = useMemo(() => highlightLine(text, compiledRules), [text, compiledRules]);

    if (!compiledRules || compiledRules.length === 0 || segments.length <= 1) {
        return <>{text}</>;
    }

    return (
        <>
            {segments.map((seg, i) =>
                seg.style ? (
                    <span key={i} style={seg.style}>
                        {seg.text}
                    </span>
                ) : (
                    seg.text
                ),
            )}
        </>
    );
}

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
    consolePopoutInstanceId,
    setConsolePopoutInstanceId,
    highlightRules,
}) {
    const { t } = useTranslation('gui');

    // 하이라이팅 규칙 컴파일 (모듈 이름 기준 캐싱)
    const compiledRules = useMemo(
        () => (highlightRules ? getCachedRules(consoleServer?.name || '_', highlightRules) : []),
        [highlightRules, consoleServer?.name],
    );

    if (!consoleServer || consolePopoutInstanceId) return null;

    return (
        <div className="console-panel">
            <div className="console-header">
                <span className="console-title">
                    <span className="console-icon">{'>'}_</span>
                    {consoleServer.name}
                </span>
                <div className="console-header-actions">
                    <button
                        className="console-popout-btn"
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
                        title={t('console.popout', { defaultValue: 'Pop out to separate window' })}
                    >
                        <Icon name="external-link" size="sm" />
                    </button>
                    <button className="console-close" onClick={closeConsole} title="Close">
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
                <input
                    type="text"
                    className="console-input"
                    value={consoleInput}
                    onChange={(e) => setConsoleInput(e.target.value)}
                    onKeyDown={(e) => {
                        if (e.key === 'Enter') sendConsoleCommand();
                    }}
                    placeholder={t('console.input_placeholder')}
                    autoFocus
                />
                <button className="console-send" onClick={sendConsoleCommand}>
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
}) {
    const { t } = useTranslation('gui');

    const compiledRules = useMemo(
        () => (highlightRules ? getCachedRules(popoutParams?.name || '_', highlightRules) : []),
        [highlightRules, popoutParams?.name],
    );

    return (
        <div className="App console-popout-app">
            <div className="console-popout-titlebar">
                <span className="console-popout-titlebar-title">
                    <span className="console-icon">{'>'}_</span>
                    {popoutParams.name}
                </span>
                <div className="console-popout-titlebar-controls">
                    <button
                        className="console-popout-titlebar-btn"
                        onClick={() => window.electron?.minimizeWindow()}
                        title={t('title_bar.minimize')}
                    >
                        ─
                    </button>
                    <button
                        className="console-popout-titlebar-btn console-popout-titlebar-close"
                        onClick={() => window.electron?.closeWindow()}
                        title={t('title_bar.close')}
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
                    <input
                        type="text"
                        className="console-input"
                        value={consoleInput}
                        onChange={(e) => setConsoleInput(e.target.value)}
                        onKeyDown={(e) => {
                            if (e.key === 'Enter') sendConsoleCommand();
                        }}
                        placeholder={t('console.input_placeholder')}
                        autoFocus
                    />
                    <button className="console-send" onClick={sendConsoleCommand}>
                        {t('console.send')}
                    </button>
                </div>
            </div>
        </div>
    );
}
