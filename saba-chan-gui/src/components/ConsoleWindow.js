import clsx from 'clsx';
import { useCallback, useEffect, useMemo, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { TITLEBAR_HEIGHT } from '../constants/layout';
import { useSettingsStore } from '../stores/useSettingsStore';
import { getCachedRules, highlightLine } from '../utils/syntaxHighlight';
import { Icon } from './index';

/**
 * Inline highlighted content renderer (shared with ConsoleView).
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

// Minimum window dimensions
const MIN_WIDTH = 360;
const MIN_HEIGHT = 200;

/**
 * ConsoleWindow — A floating, draggable, resizable console window that lives
 * inside the main app viewport (desktop-style window management).
 */
export function ConsoleWindow({
    instanceId,
    state, // { server, lines, input, minimized, pinned, position, size, zIndex }
    focusConsole,
    minimizeConsole,
    closeConsole,
    togglePin,
    updatePosition,
    updateSize,
    setConsoleInput,
    sendConsoleCommand,
    setConsolePopoutInstanceId,
    highlightRules,
    servers,
}) {
    const { t } = useTranslation('gui');
    const windowRef = useRef(null);
    const consoleEndRef = useRef(null);
    const dragRef = useRef(null);
    const resizeRef = useRef(null);
    const syntaxEnabled = useSettingsStore((s) => s.consoleSyntaxHighlight);

    const compiledRules = useMemo(
        () => (syntaxEnabled && highlightRules ? getCachedRules(state.server?.name || '_', highlightRules) : []),
        [highlightRules, state.server?.name, syntaxEnabled],
    );

    // Auto-scroll on new lines
    useEffect(() => {
        if (consoleEndRef.current && !state.minimized) {
            consoleEndRef.current.scrollIntoView({ behavior: 'smooth' });
        }
    }, [state.lines, state.minimized]);

    // ── Drag handling ───────────────────────────────────────

    const handleDragStart = useCallback(
        (e) => {
            // Only left mouse button
            if (e.button !== 0) return;
            e.preventDefault();
            focusConsole(instanceId);

            const startX = e.clientX;
            const startY = e.clientY;
            const startPos = { ...state.position };

            const handleDragMove = (moveEvent) => {
                const mx = Math.max(0, Math.min(moveEvent.clientX, window.innerWidth));
                const my = Math.max(0, Math.min(moveEvent.clientY, window.innerHeight));
                const dx = mx - startX;
                const dy = my - startY;
                updatePosition(instanceId, {
                    x: startPos.x + dx,
                    y: Math.max(TITLEBAR_HEIGHT, startPos.y + dy),
                });
            };

            const handleDragEnd = () => {
                document.removeEventListener('mousemove', handleDragMove);
                document.removeEventListener('mouseup', handleDragEnd);
            };

            document.addEventListener('mousemove', handleDragMove);
            document.addEventListener('mouseup', handleDragEnd);
        },
        [instanceId, state.position, focusConsole, updatePosition],
    );

    // ── Resize handling ─────────────────────────────────────

    const handleResizeStart = useCallback(
        (e, direction) => {
            if (e.button !== 0) return;
            e.preventDefault();
            e.stopPropagation();
            focusConsole(instanceId);

            const startX = e.clientX;
            const startY = e.clientY;
            const startSize = { ...state.size };
            const startPos = { ...state.position };

            const handleResizeMove = (moveEvent) => {
                const mx = Math.max(0, Math.min(moveEvent.clientX, window.innerWidth));
                const my = Math.max(0, Math.min(moveEvent.clientY, window.innerHeight));
                const dx = mx - startX;
                const dy = my - startY;

                let newWidth = startSize.width;
                let newHeight = startSize.height;
                let newX = startPos.x;
                let newY = startPos.y;

                if (direction.includes('e')) {
                    newWidth = Math.max(MIN_WIDTH, startSize.width + dx);
                }
                if (direction.includes('w')) {
                    const proposedWidth = startSize.width - dx;
                    if (proposedWidth >= MIN_WIDTH) {
                        newWidth = proposedWidth;
                        newX = startPos.x + dx;
                    }
                }
                if (direction.includes('s')) {
                    newHeight = Math.max(MIN_HEIGHT, startSize.height + dy);
                }
                if (direction.includes('n')) {
                    const proposedHeight = startSize.height - dy;
                    if (proposedHeight >= MIN_HEIGHT) {
                        newHeight = proposedHeight;
                        newY = Math.max(TITLEBAR_HEIGHT, startPos.y + dy);
                    }
                }

                updateSize(instanceId, { width: newWidth, height: newHeight });
                updatePosition(instanceId, { x: newX, y: newY });
            };

            const handleResizeEnd = () => {
                document.removeEventListener('mousemove', handleResizeMove);
                document.removeEventListener('mouseup', handleResizeEnd);
            };

            document.addEventListener('mousemove', handleResizeMove);
            document.addEventListener('mouseup', handleResizeEnd);
        },
        [instanceId, state.size, state.position, focusConsole, updateSize, updatePosition],
    );

    if (state.minimized) return null;

    return (
        <div
            ref={windowRef}
            className={`console-window${state.pinned ? ' cw-pinned' : ''}`}
            style={{
                left: state.position.x,
                top: state.position.y,
                width: state.size.width,
                height: state.size.height,
                zIndex: state.zIndex,
            }}
            onMouseDown={() => focusConsole(instanceId)}
        >
            {/* Resize handles */}
            <div className="cw-resize cw-resize-n" onMouseDown={(e) => handleResizeStart(e, 'n')} />
            <div className="cw-resize cw-resize-s" onMouseDown={(e) => handleResizeStart(e, 's')} />
            <div className="cw-resize cw-resize-e" onMouseDown={(e) => handleResizeStart(e, 'e')} />
            <div className="cw-resize cw-resize-w" onMouseDown={(e) => handleResizeStart(e, 'w')} />
            <div className="cw-resize cw-resize-ne" onMouseDown={(e) => handleResizeStart(e, 'ne')} />
            <div className="cw-resize cw-resize-nw" onMouseDown={(e) => handleResizeStart(e, 'nw')} />
            <div className="cw-resize cw-resize-se" onMouseDown={(e) => handleResizeStart(e, 'se')} />
            <div className="cw-resize cw-resize-sw" onMouseDown={(e) => handleResizeStart(e, 'sw')} />

            {/* Title bar — draggable */}
            <div className="cw-titlebar" onMouseDown={handleDragStart}>
                <span className="cw-title">
                    <span className="console-icon">{'>'}_</span>
                    {state.server.name}
                </span>
                <div className="cw-controls">
                    <button
                        className={`cw-btn cw-btn-pin${state.pinned ? ' cw-btn-pinned' : ''}`}
                        onClick={(e) => {
                            e.stopPropagation();
                            togglePin(instanceId);
                        }}
                        title={state.pinned ? t('console.unpin', { defaultValue: 'Unpin' }) : t('console.pin', { defaultValue: 'Pin on top' })}
                    >
                        <Icon name="pin" size="sm" />
                    </button>
                    <button
                        className="cw-btn cw-btn-minimize"
                        onClick={(e) => {
                            e.stopPropagation();
                            minimizeConsole(instanceId);
                        }}
                        title={t('console.minimize', { defaultValue: 'Minimize to dock' })}
                    >
                        <Icon name="minus" size="sm" />
                    </button>
                    <button
                        className="cw-btn cw-btn-popout"
                        onClick={async (e) => {
                            e.stopPropagation();
                            try {
                                const result = await window.api.consolePopout(instanceId, state.server.name);
                                if (result?.ok) setConsolePopoutInstanceId(instanceId);
                                closeConsole(instanceId);
                            } catch (err) {
                                console.error('Popout failed:', err);
                            }
                        }}
                        title={t('console.popout', { defaultValue: 'Pop out to separate window' })}
                    >
                        <Icon name="external-link" size="sm" />
                    </button>
                    <button
                        className="cw-btn cw-btn-close"
                        onClick={(e) => {
                            e.stopPropagation();
                            closeConsole(instanceId);
                        }}
                        title={t('modals.close')}
                    >
                        &times;
                    </button>
                </div>
            </div>

            {/* Console output */}
            <div className="cw-output">
                {state.lines.length === 0 && <div className="console-empty">{t('console.waiting')}</div>}
                {state.lines.map((line) => (
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

            {/* Input row */}
            <div className="console-input-row">
                <span className="console-prompt">{'>'}</span>
                <div className={clsx('console-input-wrap', state.stdinDisabled && 'console-input-disabled')}>
                    {state.stdinDisabled && <Icon name="alertTriangle" size="sm" />}
                    <input
                        type="text"
                        className="console-input"
                        value={state.input}
                        onChange={(e) => setConsoleInput(instanceId, e.target.value)}
                        onKeyDown={(e) => {
                            if (e.key === 'Enter') sendConsoleCommand(instanceId);
                        }}
                        placeholder={state.stdinDisabled ? t('console.stdin_disabled') : t('console.input_placeholder')}
                        disabled={state.stdinDisabled}
                    />
                </div>
                <button className="console-send" onClick={() => sendConsoleCommand(instanceId)} disabled={state.stdinDisabled}>
                    {t('console.send')}
                </button>
            </div>
        </div>
    );
}
