import clsx from 'clsx';
import { useCallback, useEffect, useMemo, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { TITLEBAR_HEIGHT } from '../constants/layout';
import { useSettingsStore } from '../stores/useSettingsStore';
import { getCachedRules } from '../utils/syntaxHighlight';
import { Icon } from './index';
import { HighlightedContent } from './HighlightedContent';

// Minimum window dimensions
const MIN_WIDTH = 360;
const MIN_HEIGHT = 200;
const RESIZE_EDGES = { n: 'top', s: 'bottom', e: 'right', w: 'left',
    ne: 'top right', nw: 'top left', se: 'bottom right', sw: 'bottom left' };

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
}) {
    const { t } = useTranslation('gui');
    const consoleEndRef = useRef(null);
    const interactionCleanup = useRef(null);
    const syntaxEnabled = useSettingsStore((s) => s.consoleSyntaxHighlight);

    const compiledRules = useMemo(
        () => (syntaxEnabled && highlightRules ? getCachedRules(state.server?.name || '_', highlightRules) : []),
        [highlightRules, state.server?.name, syntaxEnabled],
    );

    useEffect(() => {
        if (!state.minimized && state.lines.length) consoleEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }, [state.lines, state.minimized]);
    useEffect(() => () => interactionCleanup.current?.(), []);

    const changeWindow = useCallback((dx, dy, direction = '', position = state.position, size = state.size) => {
        if (!direction) {
            updatePosition(instanceId, { x: position.x + dx, y: Math.max(TITLEBAR_HEIGHT, position.y + dy) });
            return;
        }
        const west = direction.includes('w');
        const north = direction.includes('n');
        const width = /[ew]/.test(direction) ? Math.max(MIN_WIDTH, size.width + (west ? -dx : dx)) : size.width;
        let height = /[ns]/.test(direction) ? Math.max(MIN_HEIGHT, size.height + (north ? -dy : dy)) : size.height;
        const x = position.x + (west ? size.width - width : 0);
        const y = north ? Math.max(TITLEBAR_HEIGHT, position.y + size.height - height) : position.y;
        if (north) height = size.height + position.y - y;
        updateSize(instanceId, { width, height });
        updatePosition(instanceId, { x, y });
    }, [instanceId, state.position, state.size, updatePosition, updateSize]);

    const startInteraction = useCallback((event, direction = '') => {
        if (event.button !== 0) return;
        event.preventDefault();
        event.stopPropagation();
        focusConsole(instanceId);
        interactionCleanup.current?.();
        const move = (next) => changeWindow(
            Math.max(0, Math.min(next.clientX, window.innerWidth)) - event.clientX,
            Math.max(0, Math.min(next.clientY, window.innerHeight)) - event.clientY,
            direction, state.position, state.size,
        );
        const end = () => {
            document.removeEventListener('mousemove', move);
            document.removeEventListener('mouseup', end);
            interactionCleanup.current = null;
        };
        interactionCleanup.current = end;
        document.addEventListener('mousemove', move);
        document.addEventListener('mouseup', end);
    }, [changeWindow, focusConsole, instanceId, state.position, state.size]);

    function handleArrow(event, direction) {
        const delta = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[event.key];
        if (!delta) return;
        event.preventDefault();
        event.stopPropagation();
        focusConsole(instanceId);
        const step = event.shiftKey ? 50 : 10;
        changeWindow(delta[0] * step, delta[1] * step, direction);
    }

    if (state.minimized) return null;

    return (
        <section aria-label={state.server.name}
            className={`console-window${state.pinned ? ' cw-pinned' : ''}`}
            style={{
                left: state.position.x,
                top: state.position.y,
                width: state.size.width,
                height: state.size.height,
                zIndex: state.zIndex,
            }}
            onMouseDown={() => focusConsole(instanceId)}
            onFocusCapture={() => focusConsole(instanceId)}
        >
            {Object.entries(RESIZE_EDGES).map(([direction, edge]) => (
                <button type="button" key={direction} className={`cw-resize cw-resize-${direction}`}
                    style={{ border: 0, padding: 0, background: 'transparent' }}
                    aria-label={t('console.resize', { defaultValue: 'Resize {{edge}} (arrow keys)', edge })}
                    onMouseDown={(event) => startInteraction(event, direction)}
                    onKeyDown={(event) => handleArrow(event, direction)} />
            ))}
            <div className="cw-titlebar">
                <button type="button" className="cw-title"
                    style={{ border: 0, padding: 0, background: 'transparent', cursor: 'inherit', flex: 1 }}
                    aria-label={t('console.move', { defaultValue: 'Move {{name}} (arrow keys)', name: state.server.name })}
                    onMouseDown={startInteraction} onKeyDown={(event) => handleArrow(event)}>
                    <span className="console-icon">{'>'}_</span>
                    {state.server.name}
                </button>
                <div className="cw-controls">
                    <button type="button"
                        className={`cw-btn cw-btn-pin${state.pinned ? ' cw-btn-pinned' : ''}`}
                        aria-label={state.pinned ? t('console.unpin') : t('console.pin')}
                        onClick={(e) => {
                            e.stopPropagation();
                            togglePin(instanceId);
                        }}
                        data-tooltip={state.pinned ? t('console.unpin', { defaultValue: 'Unpin' }) : t('console.pin', { defaultValue: 'Pin on top' })}
                    >
                        <Icon name="pin" size="sm" />
                    </button>
                    <button type="button"
                        className="cw-btn cw-btn-minimize"
                        aria-label={t('console.minimize')}
                        onClick={(e) => {
                            e.stopPropagation();
                            minimizeConsole(instanceId);
                        }}
                        data-tooltip={t('console.minimize', { defaultValue: 'Minimize to dock' })}
                    >
                        <Icon name="minus" size="sm" />
                    </button>
                    <button type="button"
                        className="cw-btn cw-btn-popout"
                        aria-label={t('console.popout')}
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
                        data-tooltip={t('console.popout', { defaultValue: 'Pop out to separate window' })}
                    >
                        <Icon name="external-link" size="sm" />
                    </button>
                    <button type="button"
                        className="cw-btn cw-btn-close"
                        aria-label={t('modals.close')}
                        onClick={(e) => {
                            e.stopPropagation();
                            closeConsole(instanceId);
                        }}
                        data-tooltip={t('modals.close')}
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
                <button type="button" className="console-send" onClick={() => sendConsoleCommand(instanceId)} disabled={state.stdinDisabled}>
                    {t('console.send')}
                </button>
            </div>
        </section>
    );
}
