import clsx from 'clsx';
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useConsoleStore } from '../../stores/useConsoleStore';
import { useSettingsStore } from '../../stores/useSettingsStore';
import { needsConfirm } from '../../utils/commandRisk';
import { commandText } from '../../utils/serverView';
import { getCachedRules } from '../../utils/syntaxHighlight';
import { HighlightedContent } from '../ConsoleView';
import { Icon } from '../Icon';
import { useContextMenu } from '../ui/ContextMenu';

// 서버별 보낸 명령 기록 (탭을 오가도 남도록 모듈 범위). 앱을 다시 켜면 비운다.
const HISTORY_MAX = 50;
const histories = new Map();
const historyOf = (id) => {
    if (!histories.has(id)) histories.set(id, []);
    return histories.get(id);
};

/**
 * 콘솔 한 줄. 줄 객체는 받은 뒤 바뀌지 않으므로, 새 줄이 붙거나 입력칸을 칠 때
 * 이미 그린 줄(최대 수천 개)을 다시 그리지 않는다.
 */
const ConsoleLine = memo(function ConsoleLine({ line, syntaxEnabled, compiledRules, stamp }) {
    return (
        <div
            className={clsx(
                'console-line',
                `console-${line.source?.toLowerCase() || 'stdout'}`,
                `console-level-${line.level?.toLowerCase() || 'info'}`,
            )}
        >
            {stamp && line.timestamp != null && <span className="console-time">{stamp(line.timestamp)}</span>}
            <HighlightedContent
                text={line.content}
                spans={syntaxEnabled ? line.spans : undefined}
                compiledRules={compiledRules}
            />
        </div>
    );
});

/** 줄 목록. 입력칸을 칠 때는 줄 배열이 그대로라 통째로 건너뛴다 */
const ConsoleLines = memo(function ConsoleLines({ lines, syntaxEnabled, compiledRules, stamp }) {
    return lines.map((line) => (
        <ConsoleLine
            key={line.key ?? line.id}
            line={line}
            syntaxEnabled={syntaxEnabled}
            compiledRules={compiledRules}
            stamp={stamp}
        />
    ));
});

/**
 * 줄 앞 시각 표시 함수. 끔이면 null. 시각은 데몬이 준 unix 초이고 이 PC 시간대·언어로 보인다
 */
export function consoleStamper(mode, withDate, lang) {
    if (!mode || mode === 'off') return null;
    const fmt = new Intl.DateTimeFormat(lang || undefined, {
        ...(withDate ? { month: '2-digit', day: '2-digit' } : {}),
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
        hour12: mode === '12h',
    });
    return (ts) => fmt.format(new Date(ts * 1000));
}

/**
 * 상세 화면에 박힌 콘솔. 창을 따로 열지 않아도 로그를 보고 명령을 보낼 수 있다(L).
 * 상태는 useMultiConsole이 들고 있고, 이 컴포넌트는 그 한 칸을 그린다.
 * `readOnly`: 콘솔 입력을 받지 않는 서버(REST 전용). 출력만 보이고 명령은 명령 탭에서 보낸다.
 */
export function ServerConsole({
    server,
    state,
    highlightRules,
    quickCommands = [],
    openConsole,
    setConsoleInput,
    sendConsoleCommand,
    poppedOut,
    onPopout,
    onPopin,
    autoFocus = false,
    readOnly = false,
}) {
    const { t, i18n } = useTranslation('gui');
    const syntaxEnabled = useSettingsStore((s) => s.consoleSyntaxHighlight);
    const stampMode = useSettingsStore((s) => s.consoleTimestamps);
    const stampDate = useSettingsStore((s) => s.consoleTimestampDate);
    const stamp = useMemo(
        () => consoleStamper(stampMode, stampDate, i18n.language),
        [stampMode, stampDate, i18n.language],
    );
    const outputRef = useRef(null);
    const inputRef = useRef(null);
    const stickRef = useRef(true);
    const cursorRef = useRef(-1); // 기록에서 몇 번째를 보고 있는지 (-1 = 새로 입력 중)
    const draftRef = useRef('');
    const [focused, setFocused] = useState(false);
    const [confirmText, setConfirmText] = useState(null); // 위험 명령 확인 대기

    // 탭이 보이면 로그 폴링 시작 (이미 열려 있으면 그대로)
    useEffect(() => {
        if (!state && !poppedOut) openConsole(server.id, server.name);
    }, [server.id, server.name, state, poppedOut, openConsole]);

    const compiledRules = useMemo(
        () => (syntaxEnabled && highlightRules ? getCachedRules(server.module || '_', highlightRules) : []),
        [syntaxEnabled, highlightRules, server.module],
    );

    const lines = state?.lines || [];

    const openMenu = useContextMenu();
    const outputMenu = (e) => {
        const sel = window.getSelection();
        const text = String(sel || '');
        const inside = !!text && !!outputRef.current?.contains(sel.anchorNode);
        openMenu(e, [
            {
                id: 'copy',
                label: t('context_menu.copy'),
                icon: 'copy',
                
                disabled: !inside,
                onSelect: () => navigator.clipboard?.writeText(text),
            },
            {
                id: 'select_all',
                label: t('context_menu.select_all'),
                icon: 'selectAll',
                onSelect: () => {
                    const range = document.createRange();
                    range.selectNodeContents(outputRef.current);
                    sel.removeAllRanges();
                    sel.addRange(range);
                },
            },
            'separator',
            {
                // 화면에서만 지운다: 데몬은 이어서 새 줄만 보내므로 지운 줄이 다시 붙지 않는다
                id: 'clear',
                label: t('context_menu.clear_console'),
                icon: 'eraser',
                disabled: lines.length === 0,
                onSelect: () =>
                    useConsoleStore
                        .getState()
                        .setConsoles((all) => (all[server.id] ? { ...all, [server.id]: { ...all[server.id], lines: [] } } : all)),
            },
        ]);
    };

    // 따로 띄운 창은 명령을 치려고 여는 창이라 열리자마자 입력칸에 커서를 둔다
    useEffect(() => {
        if (autoFocus) inputRef.current?.focus();
    }, [autoFocus]);

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

    const stdinDisabled = readOnly || state?.stdinDisabled || server.status !== 'running';

    return (
        <div className="term">
            <div
                className="term-output"
                ref={outputRef}
                onScroll={(e) => {
                    const el = e.currentTarget;
                    stickRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
                }}
                onContextMenu={outputMenu}
                role="log"
                aria-live="polite"
            >
                {lines.length === 0 && (
                    <div className="term-empty">
                        {server.status === 'running' ? t('console.waiting') : t('shell.console_offline')}
                    </div>
                )}
                <ConsoleLines
                    lines={lines}
                    syntaxEnabled={syntaxEnabled}
                    compiledRules={compiledRules}
                    stamp={stamp}
                />
            </div>
            {!stdinDisabled && quickCommands.length > 0 && (
                <div className="term-quick" role="group" aria-label={t('shell.console_quick')}>
                    {quickCommands.slice(0, 6).map((c) => (
                        <button
                            key={c.name}
                            type="button"
                            className={clsx('term-chip', c.risk && `risk-${c.risk}`)}
                            data-tooltip={commandText(t, server.module, c).description || c.name}
                            onClick={() => {
                                // 바로 보내지 않고 채워만 둔다: 인자가 필요하거나 위험한 명령도 있으니까
                                setConsoleInput(server.id, c.inputs?.length ? `${c.name} ` : c.name);
                                inputRef.current?.focus();
                            }}
                        >
                            {commandText(t, server.module, c).label}
                        </button>
                    ))}
                </div>
            )}
            {confirmText && confirmText === (state?.input || '').trim() && (
                <div className="notice tone-stop risk-confirm term-confirm" role="alert">
                    <Icon name="alertTriangle" size="sm" />
                    <span className="risk-confirm-text">{t('shell.risk_confirm', { command: confirmText })}</span>
                    <button
                        type="button"
                        className="btn btn-danger btn-sm"
                        onClick={() => {
                            const hist = historyOf(server.id);
                            if (hist[hist.length - 1] !== confirmText) hist.push(confirmText);
                            setConfirmText(null);
                            sendConsoleCommand(server.id);
                        }}
                    >
                        {t('shell.risk_run')}
                    </button>
                    <button type="button" className="btn btn-ghost btn-sm" onClick={() => setConfirmText(null)}>
                        {t('common:cancel', { defaultValue: '취소' })}
                    </button>
                </div>
            )}
            <form
                className={clsx('term-input', { focused, disabled: stdinDisabled })}
                onSubmit={(e) => {
                    e.preventDefault();
                    const cmd = (state?.input || '').trim();
                    if (!cmd || stdinDisabled) return;
                    // 되돌리기 어려운 명령은 한 번 더 확인한다 (module.toml risk = "danger")
                    if (needsConfirm(quickCommands, cmd) && confirmText !== cmd) {
                        setConfirmText(cmd);
                        return;
                    }
                    setConfirmText(null);
                    const hist = historyOf(server.id);
                    if (hist[hist.length - 1] !== cmd) hist.push(cmd);
                    if (hist.length > HISTORY_MAX) hist.shift();
                    cursorRef.current = -1;
                    sendConsoleCommand(server.id);
                }}
            >
                <Icon name="chevronRight" size="sm" />
                <input
                    ref={inputRef}
                    type="text"
                    className="term-field"
                    value={state?.input || ''}
                    onChange={(e) => {
                        cursorRef.current = -1;
                        setConsoleInput(server.id, e.target.value);
                    }}
                    onFocus={() => setFocused(true)}
                    onBlur={() => setFocused(false)}
                    onKeyDown={(e) => {
                        // ↑/↓ 로 보냈던 명령을 다시 꺼낸다 (터미널과 같은 손버릇)
                        const hist = historyOf(server.id);
                        if (!hist.length || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
                        e.preventDefault();
                        if (cursorRef.current === -1) draftRef.current = state?.input || '';
                        const next =
                            e.key === 'ArrowUp'
                                ? cursorRef.current === -1
                                    ? hist.length - 1
                                    : Math.max(0, cursorRef.current - 1)
                                : cursorRef.current === -1 || cursorRef.current >= hist.length - 1
                                  ? -1
                                  : cursorRef.current + 1;
                        cursorRef.current = next;
                        setConsoleInput(server.id, next === -1 ? draftRef.current : hist[next]);
                    }}
                    placeholder={
                        readOnly
                            ? t('shell.console_read_only')
                            : server.status !== 'running'
                              ? t('shell.console_offline_input')
                              : state?.stdinDisabled
                                ? t('console.stdin_disabled')
                                : t('console.input_placeholder')
                    }
                    disabled={stdinDisabled}
                    aria-label={t('console.input_placeholder')}
                    autoComplete="off"
                    spellCheck={false}
                />
                {!stdinDisabled && <span className="term-hint">{t('shell.console_hint')}</span>}
                <button
                    type="submit"
                    className="term-send"
                    disabled={stdinDisabled || !state?.input?.trim()}
                    data-tooltip={t('console.send')}
                    aria-label={t('console.send')}
                >
                    <Icon name="arrowUp" size="sm" />
                </button>
                {onPopout && (
                    <>
                        <span className="term-divider" aria-hidden="true" />
                        <button
                            type="button"
                            className="icon-btn term-popout"
                            onClick={onPopout}
                            data-tooltip={t('console.popout')}
                            aria-label={t('console.popout')}
                        >
                            <Icon name="externalLink" size="sm" />
                        </button>
                    </>
                )}
            </form>
        </div>
    );
}
