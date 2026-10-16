import clsx from 'clsx';
import { useEffect, useLayoutEffect, useMemo, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { useSettingsStore } from '../../stores/useSettingsStore';
import { getCachedRules } from '../../utils/syntaxHighlight';
import { HighlightedContent } from '../ConsoleView';
import { Icon } from '../Icon';

/**
 * 상세 화면에 박힌 콘솔. 창을 따로 열지 않아도 로그를 보고 명령을 보낼 수 있다(L).
 * 상태는 useMultiConsole이 들고 있고, 이 컴포넌트는 그 한 칸을 그린다.
 */
export function ServerConsole({
    server,
    state,
    highlightRules,
    openConsole,
    setConsoleInput,
    sendConsoleCommand,
    poppedOut,
    onPopout,
    onPopin,
}) {
    const { t } = useTranslation('gui');
    const syntaxEnabled = useSettingsStore((s) => s.consoleSyntaxHighlight);
    const outputRef = useRef(null);
    const stickRef = useRef(true);

    // 탭이 보이면 로그 폴링 시작 (이미 열려 있으면 그대로)
    useEffect(() => {
        if (!state && !poppedOut) openConsole(server.id, server.name);
    }, [server.id, server.name, state, poppedOut, openConsole]);

    const compiledRules = useMemo(
        () => (syntaxEnabled && highlightRules ? getCachedRules(server.module || '_', highlightRules) : []),
        [syntaxEnabled, highlightRules, server.module],
    );

    const lines = state?.lines || [];

    // 맨 아래를 보고 있을 때만 새 줄을 따라 내려간다
    // biome-ignore lint/correctness/useExhaustiveDependencies: 줄 수가 바뀔 때만 스크롤
    useLayoutEffect(() => {
        const el = outputRef.current;
        if (el && stickRef.current) el.scrollTop = el.scrollHeight;
    }, [lines.length]);

    if (poppedOut) {
        return (
            <div className="term term-placeholder">
                <Icon name="externalLink" size="lg" />
                <p>{t('shell.console_popped_out')}</p>
                <button type="button" className="btn btn-ghost" onClick={onPopin}>
                    {t('shell.console_popin')}
                </button>
            </div>
        );
    }

    const stdinDisabled = state?.stdinDisabled || server.status !== 'running';

    return (
        <div className="term">
            <div
                className="term-output"
                ref={outputRef}
                onScroll={(e) => {
                    const el = e.currentTarget;
                    stickRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
                }}
                role="log"
                aria-live="polite"
            >
                {lines.length === 0 && (
                    <div className="term-empty">
                        {server.status === 'running' ? t('console.waiting') : t('shell.console_offline')}
                    </div>
                )}
                {lines.map((line) => (
                    <div
                        key={line.id}
                        className={clsx(
                            'console-line',
                            `console-${line.source?.toLowerCase() || 'stdout'}`,
                            `console-level-${line.level?.toLowerCase() || 'info'}`,
                        )}
                    >
                        <HighlightedContent text={line.content} compiledRules={compiledRules} />
                    </div>
                ))}
            </div>
            <form
                className="term-input"
                onSubmit={(e) => {
                    e.preventDefault();
                    sendConsoleCommand(server.id);
                }}
            >
                <span className="term-prompt" aria-hidden="true">
                    {'>'}
                </span>
                <input
                    type="text"
                    className="term-field"
                    value={state?.input || ''}
                    onChange={(e) => setConsoleInput(server.id, e.target.value)}
                    placeholder={
                        server.status !== 'running'
                            ? t('shell.console_offline_input')
                            : state?.stdinDisabled
                              ? t('console.stdin_disabled')
                              : t('console.input_placeholder')
                    }
                    disabled={stdinDisabled}
                    aria-label={t('console.input_placeholder')}
                />
                <button type="submit" className="btn btn-primary btn-sm" disabled={stdinDisabled || !state?.input}>
                    {t('console.send')}
                </button>
                <button
                    type="button"
                    className="icon-btn"
                    onClick={onPopout}
                    title={t('console.popout')}
                    aria-label={t('console.popout')}
                >
                    <Icon name="externalLink" size="sm" />
                </button>
            </form>
        </div>
    );
}
