import clsx from 'clsx';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSettingsStore } from '../stores/useSettingsStore';
import { getCachedRules, highlightLine } from '../utils/syntaxHighlight';
import { Icon } from './index';

/**
 * 단일 콘솔 라인의 content를 하이라이팅하여 렌더링
 */
export function HighlightedContent({ text, compiledRules }) {
    const segments = useMemo(() => highlightLine(text, compiledRules), [text, compiledRules]);

    // 한 줄 전체가 한 규칙에 걸리면 조각이 하나뿐이지만 스타일은 있다
    if (!compiledRules || compiledRules.length === 0 || (segments.length <= 1 && !segments[0]?.style)) {
        return <>{text}</>;
    }

    // 조각의 줄 안 시작 위치를 key로 쓴다 (같은 줄 안에서 겹치지 않는다)
    let offset = 0;
    return (
        <>
            {segments.map((seg) => {
                const at = offset;
                offset += seg.text.length;
                return seg.style ? (
                    <span key={at} style={seg.style}>
                        {seg.text}
                    </span>
                ) : (
                    seg.text
                );
            })}
        </>
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
    // 팝아웃 창은 명령을 치려고 여는 창이라 열리자마자 입력칸에 커서를 둔다
    const inputRef = useRef(null);
    useEffect(() => inputRef.current?.focus(), []);

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
                    <button
                        type="button"
                        className={`console-popout-titlebar-btn console-popout-titlebar-popin`}
                        onClick={handlePopin}
                        title={t('console.popin', { defaultValue: 'Bring back to main window' })}
                    >
                        <Icon name="external-link-in" size="sm" />
                    </button>
                    <button
                        type="button"
                        className={`console-popout-titlebar-btn console-popout-titlebar-pin${pinned ? ' console-popout-titlebar-pinned' : ''}`}
                        onClick={handleTogglePin}
                        title={
                            pinned
                                ? t('console.unpin', { defaultValue: 'Unpin' })
                                : t('console.pin', { defaultValue: 'Pin on top' })
                        }
                    >
                        <Icon name="pin" size="sm" />
                    </button>
                    <button
                        type="button"
                        className="console-popout-titlebar-btn"
                        onClick={() => window.electron?.minimizeWindow()}
                        title={t('title_bar.minimize')}
                    >
                        ─
                    </button>
                    <button
                        type="button"
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
                            ref={inputRef}
                            disabled={stdinDisabled}
                        />
                    </div>
                    <button
                        type="button"
                        className="console-send"
                        onClick={sendConsoleCommand}
                        disabled={stdinDisabled}
                    >
                        {t('console.send')}
                    </button>
                </div>
            </div>
        </div>
    );
}
