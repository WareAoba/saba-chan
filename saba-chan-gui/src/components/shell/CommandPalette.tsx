import clsx from 'clsx';
import { type MutableRefObject, useEffect, useRef, useState } from 'react';
import { OPEN_SONAR_EVENT } from '../guides/guides';
import { Icon } from '../Icon';
import { CATEGORIES, type Group, type PaletteItem, type Translate } from './paletteModel';
import { commandPreview, iconOf, itemPreview, optional, paletteFoot, statusText } from './paletteView';
import { usePalette } from './usePalette';

/** plan이 돌려주는 요약 한 줄 */
interface PlanSummary {
    label?: string;
    labelKey?: string;
    value?: unknown;
    before?: unknown;
}

/**
 * 명령 팔레트 (Ctrl+K, 사용자 11-10 시안 v5): Spotlight 모양의 검색 막대 + 카테고리 버튼(조작·게임 명령·바로 가기).
 * 항목 계산은 paletteModel. 바로 가기는 실행하지 않고 그 화면을 열고, 조작은 데몬 액션(plan → run),
 * 게임 명령은 지금 보낼 수 있는 것만(데몬 `/commands`의 ready) 칸을 하나씩 받아 보낸다.
 */
export const paletteAvailable = () => !!window.api?.actionsPlan && !!window.api?.actionsRun;

/** 계획 요약 값. 설정 변경은 `before`가 붙어 "이전 → 새 값"으로 보인다 (null이면 없음) */
export const summaryValue = (s: PlanSummary, t: Translate) =>
    'before' in s
        ? `${s.before == null ? t('palette.none') : String(s.before)} → ${String(s.value ?? '')}`
        : String(s.value ?? '');

function Magnifier() {
    return (
        <svg
            className="palette-mag"
            width="20"
            height="20"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2.2"
            strokeLinecap="round"
            aria-hidden="true"
        >
            <circle cx="11" cy="11" r="7" />
            <path d="m20 20-3.5-3.5" />
        </svg>
    );
}

function Palette({ onClose, busyRef }: { onClose: () => void; busyRef: MutableRefObject<boolean> }) {
    const {
        tr,
        mode,
        scope,
        leaving,
        context,
        query,
        q,
        cur,
        rowCount,
        current,
        cats,
        activeCategory,
        groups,
        flat,
        input,
        choices,
        shownChoices,
        picked,
        pickServer,
        args,
        armed,
        control,
        error,
        busyText,
        done,
        shake,
        inputRef,
        listRef,
        hlRef,
        serverOf,
        entityName,
        close,
        backToList,
        clearScope,
        acceptArg,
        editLastArg,
        commandLine,
        sendCommand,
        runControl,
        choose,
        toggleCategory,
        onKeyDown,
        typeQuery,
        expandGroup,
        setCursor,
    } = usePalette(onClose, busyRef);


    const placeholder =
        mode === 'entities'
            ? tr('palette.placeholder_entities')
            : input
              ? optional(input)
                  ? tr('palette.arg_optional', { label: input.label || input.name })
                  : input.label || input.name
              : mode === 'list'
                ? scope
                    ? tr('palette.placeholder_scope', { name: entityName(scope) })
                    : tr('palette.placeholder')
                : '';
    const ctxName = !scope && !q && context && mode === 'list' ? ` · ${entityName(context)}` : '';

    const subOf = (it: PaletteItem) => {
        const s = serverOf(it.serverId);
        switch (it.group) {
            case 'eguild':
                return tr('palette.discord_server');
            case 'esrv':
                return `${tr('palette.game_server')} · ${statusText(tr, s?.status)}`;
            case 'xcmd':
                return `${s?.name ?? ''} · ${statusText(tr, s?.status)}`;
            case 'ctl':
                return statusText(tr, s?.status);
            case 'cmd':
                return it.command?.inputs.length
                    ? it.command.inputs
                          .map((i) => (optional(i) ? `${i.label} (${tr('palette.optional')})` : i.label))
                          .join(', ')
                    : tr('palette.no_inputs');
            default:
                // 가는 길에서 제목과 같은 조각은 뺀다 (서버 탭이면 서버 이름, 설정 칸이면 서버 › 탭)
                return (it.path || []).filter((p) => p !== it.title).join(' › ');
        }
    };
    const ACT: Record<Group, string> = {
        ctl: 'palette.act.run',
        cmd: 'palette.act.input',
        xcmd: 'palette.act.switch',
        jump: 'palette.act.open',
        setting: 'palette.act.open',
        esrv: 'palette.act.scope',
        eguild: 'palette.act.scope',
    };

    const preview = (it?: PaletteItem | null) => {
        if (picked && mode === 'args') return commandPreview(picked.command, pickServer?.name, tr);
        if (!it) return null;
        return itemPreview(it, serverOf(it.serverId), tr);
    };

    const stage = mode === 'confirm' || mode === 'controlConfirm' || mode === 'busy' || mode === 'done';
    const danger = picked?.command.risk === 'danger';
    const showGo = mode === 'args' || mode === 'confirm';
    let rowIndex = 0;

    const row = (
        i: number,
        key: string,
        content: React.ReactNode,
        onPick: () => void,
        delay = Math.min(i, 14) * 18,
    ) => (
        <div
            key={key}
            id={`palette-opt-${i}`}
            data-row={i}
            role="option"
            aria-selected={i === cur}
            tabIndex={-1}
            className={clsx('palette-row', { on: i === cur })}
            style={{ animationDelay: `${delay}ms` }}
            onMouseMove={() => i !== cur && setCursor(i)}
            onClick={onPick}
            onKeyDown={() => {}}
        >
            {content}
        </div>
    );

    return (
        // biome-ignore lint/a11y/noStaticElementInteractions: 바깥을 누르면 닫는다
        <div className="palette-overlay" onMouseDown={(e) => e.target === e.currentTarget && close()}>
            <div
                className="palette"
                role="dialog"
                aria-modal="true"
                aria-label={tr('palette.title')}
                onKeyDown={onKeyDown}
            >
                <div className="palette-top">
                    <div key={shake} className={clsx('palette-bar palette-glass', { shake: shake > 0 })}>
                        <Magnifier />
                        <div className="palette-tokens">
                            {scope && (
                                <button
                                    type="button"
                                    className={clsx('palette-tok scope', { leaving })}
                                    aria-label={`${entityName(scope)} · ${tr('palette.chip_clear')}`}
                                    onClick={() => {
                                        if (mode !== 'list') backToList();
                                        clearScope();
                                    }}
                                >
                                    {scope.kind === 'server' ? (
                                        <span className={clsx('palette-dot', serverOf(scope.id)?.status)} />
                                    ) : (
                                        <span className="palette-gmark">{Array.from(entityName(scope))[0]}</span>
                                    )}
                                    {entityName(scope)}
                                    <span className="x" aria-hidden="true">
                                        ×
                                    </span>
                                </button>
                            )}
                            {picked && mode !== 'list' && (
                                <span className="palette-tok">
                                    {scope ? '' : `${pickServer?.name ?? ''} · `}
                                    {picked.command.label}
                                </span>
                            )}
                            {picked &&
                                mode !== 'list' &&
                                args.map((a, i) => (
                                    // biome-ignore lint/suspicious/noArrayIndexKey: 칸 순서대로 쌓인다
                                    <span key={i} className="palette-tok arg">
                                        <em>{picked.command.inputs[i]?.label}</em>
                                        {a.label}
                                    </span>
                                ))}
                        </div>
                        <input
                            ref={inputRef}
                            // biome-ignore lint/a11y/noAutofocus: 팔레트를 열면 바로 친다
                            autoFocus
                            value={query}
                            readOnly={stage}
                            onChange={(e) => typeQuery(e.target.value)}
                            placeholder={placeholder}
                            aria-label={tr('palette.search_label')}
                            role="combobox"
                            aria-expanded="true"
                            aria-controls="palette-list"
                            aria-activedescendant={rowCount && !stage ? `palette-opt-${cur}` : undefined}
                            spellCheck={false}
                            autoComplete="off"
                            type={input?.type === 'password' ? 'password' : 'text'}
                        />
                        {showGo && (
                            <button
                                type="button"
                                className={clsx('palette-go', { danger: mode === 'confirm' && danger })}
                                onClick={() => (mode === 'confirm' ? sendCommand() : acceptArg())}
                            >
                                {mode === 'args'
                                    ? `${tr('palette.next')} ↵`
                                    : armed
                                      ? `${tr('palette.again')} ↵`
                                      : danger
                                        ? `${picked?.command.label} ↵`
                                        : `${tr('palette.send')} ↵`}
                            </button>
                        )}
                    </div>
                    <div className="palette-cats">
                        {CATEGORIES.filter((c) => cats.includes(c)).map((c) => (
                            <button
                                key={c}
                                type="button"
                                className={clsx('palette-cat palette-glass', { on: activeCategory === c })}
                                aria-label={tr(`palette.category.${c}`)}
                                aria-pressed={activeCategory === c}
                                onClick={() => toggleCategory(c)}
                            >
                                <Icon
                                    name={c === 'ctl' ? 'power' : c === 'cmd' ? 'terminal' : 'externalLink'}
                                    size="md"
                                />
                                <span className="tip">{tr(`palette.category.${c}`)}</span>
                            </button>
                        ))}
                    </div>
                </div>

                <div className={clsx('palette-res palette-glass', { single: stage })}>
                    {stage ? (
                        <div className="palette-stage">
                            {mode === 'busy' && (
                                <div className="palette-done">
                                    <div className="palette-spin" />
                                    <span className="muted">{busyText}</span>
                                </div>
                            )}
                            {mode === 'done' && done && (
                                <div className="palette-done ok">
                                    <svg
                                        width="64"
                                        height="64"
                                        viewBox="0 0 64 64"
                                        fill="none"
                                        stroke="currentColor"
                                        strokeWidth="4"
                                        strokeLinecap="round"
                                        strokeLinejoin="round"
                                        aria-hidden="true"
                                    >
                                        <circle cx="32" cy="32" r="25" />
                                        <path d="m21 33 8 8 15-17" />
                                    </svg>
                                    <b>{done.title}</b>
                                    {done.line && <span className="palette-cmdline">{done.line}</span>}
                                </div>
                            )}
                            {mode === 'confirm' && picked && (
                                <>
                                    <h3>{tr('palette.confirm_send', { name: pickServer?.name ?? '' })}</h3>
                                    <div className="palette-cmdline">{commandLine()}</div>
                                    {danger && (
                                        <div className="palette-warn">
                                            <Icon name="alertTriangle" size="sm" /> {tr('palette.danger_warn')}
                                        </div>
                                    )}
                                    {error && <p className="palette-error">{error}</p>}
                                    <div className="palette-btns">
                                        <button
                                            type="button"
                                            className="palette-btn"
                                            onClick={() => (args.length ? editLastArg() : backToList())}
                                        >
                                            {tr('palette.edit')}
                                        </button>
                                        <button
                                            type="button"
                                            className={clsx('palette-btn', danger ? 'dan' : 'pri', { armed })}
                                            onClick={sendCommand}
                                        >
                                            {armed
                                                ? tr('palette.send_again')
                                                : danger
                                                  ? picked.command.label
                                                  : tr('palette.send')}
                                        </button>
                                    </div>
                                </>
                            )}
                            {mode === 'controlConfirm' && control && (
                                <>
                                    <h3>
                                        {tr('palette.confirm_title', {
                                            action: `${serverOf(control.item.serverId)?.name ?? ''} ${control.item.title}`,
                                        })}
                                    </h3>
                                    {Array.isArray(control.plan.summary) && control.plan.summary.length > 0 && (
                                        <dl className="palette-summary">
                                            {control.plan.summary.map((s: PlanSummary, i: number) => (
                                                // biome-ignore lint/suspicious/noArrayIndexKey: 읽기 전용 목록
                                                <div key={i} className="palette-dl-row">
                                                    <dt>{s.label || s.labelKey}</dt>
                                                    <dd>{summaryValue(s, tr)}</dd>
                                                </div>
                                            ))}
                                        </dl>
                                    )}
                                    {error && <p className="palette-error">{error}</p>}
                                    <div className="palette-btns">
                                        <button type="button" className="palette-btn" onClick={backToList}>
                                            {tr('modals.cancel')}
                                        </button>
                                        <button
                                            type="button"
                                            className={clsx(
                                                'palette-btn',
                                                control.item.control === 'stop' || control.item.control === 'restart'
                                                    ? 'dan'
                                                    : 'pri',
                                            )}
                                            onClick={() => runControl(control.item, control.plan)}
                                        >
                                            {control.item.title}
                                        </button>
                                    </div>
                                </>
                            )}
                        </div>
                    ) : (
                        <>
                            <div
                                className="palette-list"
                                id="palette-list"
                                role="listbox"
                                aria-label={tr('palette.title')}
                                ref={listRef}
                            >
                                <div className="palette-hl" ref={hlRef} />
                                {mode === 'args' && input ? (
                                    <>
                                        <div className="palette-grp">
                                            {input.label}
                                            {optional(input) && <span className="n">{tr('palette.optional')}</span>}
                                        </div>
                                        {shownChoices.map((c, i) =>
                                            row(
                                                i,
                                                c.value,
                                                <>
                                                    <span className="palette-ico">{Array.from(c.label)[0]}</span>
                                                    <span className="palette-txt">
                                                        <b>{c.label}</b>
                                                        {c.label !== c.value && <small>{c.value}</small>}
                                                    </span>
                                                    <span className="palette-meta">
                                                        <span className="ret">↵ {tr('palette.act.pick')}</span>
                                                    </span>
                                                </>,
                                                () => acceptArg(i),
                                                i * 22,
                                            ),
                                        )}
                                        {!choices && (
                                            <div className="palette-hint">
                                                {tr('palette.arg_hint', { label: input.label })}
                                                {input.placeholder &&
                                                    ` ${tr('palette.example', { value: input.placeholder })}`}
                                                {input.type === 'number' && ` ${tr('palette.arg_number')}`}
                                                {optional(input) && (
                                                    <>
                                                        <br />
                                                        {tr('palette.arg_skip')}
                                                    </>
                                                )}
                                            </div>
                                        )}
                                        {choices && !shownChoices.length && (
                                            <div className="palette-none">{tr('palette.empty')}</div>
                                        )}
                                    </>
                                ) : (
                                    <>
                                        {groups.map((g) => (
                                            <div key={g.group} className="palette-section">
                                                <div className="palette-grp">
                                                    {tr(`palette.group.${g.group}`)}
                                                    {['ctl', 'cmd', 'jump'].includes(g.group) ? ctxName : ''}
                                                    <span className="n">{g.total}</span>
                                                </div>
                                                {g.items.map((it) => {
                                                    const i = rowIndex++;
                                                    const risk = it.command?.risk;
                                                    return row(
                                                        i,
                                                        it.key,
                                                        <>
                                                            {iconOf(it, serverOf(it.serverId)?.status)}
                                                            <span className="palette-txt">
                                                                <b>{it.title}</b>
                                                                <small>{subOf(it)}</small>
                                                            </span>
                                                            <span className="palette-meta">
                                                                {(risk === 'danger' || risk === 'caution') && (
                                                                    <span
                                                                        className={clsx(
                                                                            'palette-pill',
                                                                            risk === 'danger' ? 'danger' : 'warn',
                                                                        )}
                                                                    >
                                                                        {tr(`palette.risk.${risk}`)}
                                                                    </span>
                                                                )}
                                                                <span className="ret">↵ {tr(ACT[it.group])}</span>
                                                            </span>
                                                        </>,
                                                        () => {
                                                            setCursor(i);
                                                            choose(it);
                                                        },
                                                    );
                                                })}
                                                {g.total > g.items.length && (
                                                    <button
                                                        type="button"
                                                        className="palette-more"
                                                        onClick={() => expandGroup(g.group)}
                                                    >
                                                        {tr('palette.more', {
                                                            group: tr(`palette.group.${g.group}`),
                                                            count: g.total - g.items.length,
                                                        })}
                                                    </button>
                                                )}
                                            </div>
                                        ))}
                                        {!flat.length && (
                                            <div className="palette-none">
                                                {tr('palette.empty')}
                                                <br />
                                                <small>{tr('palette.empty_hint')}</small>
                                            </div>
                                        )}
                                        {error && <p className="palette-error">{error}</p>}
                                    </>
                                )}
                            </div>
                            <aside
                                className="palette-pv"
                                key={mode === 'args' ? `args:${picked?.command.name}` : (current?.key ?? '')}
                            >
                                {preview(current)}
                            </aside>
                        </>
                    )}
                    <footer className="palette-foot">{paletteFoot(mode, !!scope, tr)}</footer>
                </div>
            </div>
        </div>
    );
}

/** Ctrl+K (macOS ⌘K)로 여닫는다. 액션 IPC가 없으면 아무것도 하지 않는다 */
export function CommandPalette() {
    const [open, setOpen] = useState(false);
    const busyRef = useRef(false);

    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'k') {
                e.preventDefault();
                setOpen((o) => (o && busyRef.current ? o : !o));
            }
        };
        // 가이드가 소나를 연다
        const onOpen = () => setOpen(true);
        window.addEventListener('keydown', onKey);
        window.addEventListener(OPEN_SONAR_EVENT, onOpen);
        return () => {
            window.removeEventListener('keydown', onKey);
            window.removeEventListener(OPEN_SONAR_EVENT, onOpen);
        };
    }, []);

    return open ? (
        <Palette
            busyRef={busyRef}
            onClose={() => {
                busyRef.current = false;
                setOpen(false);
            }}
        />
    ) : null;
}
