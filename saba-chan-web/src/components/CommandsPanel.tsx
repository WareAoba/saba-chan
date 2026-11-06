import { CommandLineField, CommandLineHelp } from '@gui/components/ui/CommandLine';
import { needsConfirm } from '@gui/utils/commandRisk';
import { applySuggestion, isLineCommand, readCommandLine } from '@gui/utils/commandSyntax';
import { useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import { commandArgs, missingInputs, useCommands } from './commands';
import { Dropdown } from './Dropdown';
import { Icon } from './icons';
import { Button, Empty, errorText, failed, Section, Sheet } from './ui';
import type { CommandInput as CommandInputSpec } from '@gui/types/daemon';
import type { ApiBody, ServerPanelProps } from '../types';

type LocalCommand = ReturnType<typeof useCommands>[number];

/** 데몬이 정리한 결과의 필드 한 칸 */
interface OutputField {
    key: string;
    label: string;
    value: string;
}

/** 데몬이 정리한 명령 결과 (데몬 commands::output) */
interface CommandOutputData {
    text?: string;
    fields?: OutputField[];
    table?: { label?: string; columns: { key: string; label: string }[]; rows: string[][] };
}

/** 받은 결과 한 건 */
interface CommandResult {
    id: number;
    label: string;
    ok: boolean;
    output: CommandOutputData | null;
    text: string;
}

/** 명령 입력 하나 (module.toml `[[commands.fields.inputs]]`) */
export function CommandInput({
    input,
    value,
    onChange,
}: {
    input: CommandInputSpec;
    value: unknown;
    onChange: (value: unknown) => void;
}) {
    const label = `${input.label || input.name}${input.required ? ' *' : ''}`;
    if (input.type === 'boolean') {
        return (
            <label className="field field-row">
                <span>{label}</span>
                <input
                    type="checkbox"
                    className="switch"
                    checked={value === true || value === 'true'}
                    onChange={(e) => onChange(e.target.checked)}
                />
            </label>
        );
    }
    if (Array.isArray(input.options) && input.options.length) {
        return (
            <div className="field">
                <span>{label}</span>
                <Dropdown
                    value={String(value ?? '')}
                    onChange={onChange}
                    options={[
                        ...(input.required ? [] : [{ value: '', label: '—' }]),
                        ...input.options.map((o) => ({ value: String(o), label: String(o) })),
                    ]}
                />
            </div>
        );
    }
    return (
        <label className="field">
            <span>{label}</span>
            <input
                type={input.type === 'number' ? 'number' : input.type === 'password' ? 'password' : 'text'}
                inputMode={input.type === 'number' ? 'numeric' : undefined}
                min={input.min}
                max={input.max}
                value={String(value ?? '')}
                placeholder={input.placeholder || ''}
                autoComplete="off"
                onChange={(e) => onChange(e.target.value)}
            />
        </label>
    );
}

const defaultsOf = (cmd: LocalCommand) =>
    Object.fromEntries((cmd.inputs || []).filter((i) => i.default != null).map((i) => [i.name, i.default]));

/** 결과 글자: 게임의 실제 응답(`data.response`, REST는 `data`)을 전송 안내(`message`)보다 먼저 보인다 */
export const resultText = (res: ApiBody | null | undefined) => {
    const data = res?.data;
    const v = data?.response ?? data ?? res?.message ?? res?.text ?? res?.output ?? res?.result;
    if (v == null || v === '') return '';
    return typeof v === 'string' ? v : JSON.stringify(v);
};

/** 처음에 보이는 결과 줄 수. 넘으면 "N개 더 보기" (앱 명령 탭과 같다) */
const PREVIEW_ROWS = 8;

/** 데몬이 정리한 명령 결과 (`output`: { text } | { fields } | { table }, 데몬 commands::output) */
export function CommandOutput({ output }: { output: CommandOutputData }) {
    const { t: tg } = useTranslation('gui');
    const [all, setAll] = useState(false);
    if (typeof output.text === 'string') return <pre className="row-sub command-output">{output.text}</pre>;
    const table = output.table;
    const rows = output.fields || table?.rows || [];
    const shown = all ? rows : rows.slice(0, PREVIEW_ROWS);
    const more = rows.length - shown.length;
    return (
        <div className="row-sub command-result">
            {table?.label && (
                <div className="command-result-title">
                    {table.label} ({rows.length})
                </div>
            )}
            {output.fields && (
                <dl className="command-result-fields">
                    {(shown as OutputField[]).map((f) => (
                        <div key={f.key} className="command-result-field">
                            <dt>{f.label}</dt>
                            <dd>{f.value}</dd>
                        </div>
                    ))}
                </dl>
            )}
            {table && shown.length > 0 && (
                <div className="command-result-scroll">
                    <table className="command-result-table">
                        <thead>
                            <tr>
                                {table.columns.map((c) => (
                                    <th key={c.key}>{c.label}</th>
                                ))}
                            </tr>
                        </thead>
                        <tbody>
                            {(shown as string[][]).map((row, i) => (
                                // biome-ignore lint/suspicious/noArrayIndexKey: 서버가 준 순서 그대로인 결과 줄 (고유 키가 없다)
                                <tr key={i}>
                                    {row.map((cell, j) => (
                                        // biome-ignore lint/suspicious/noArrayIndexKey: 열 순서 고정
                                        <td key={j}>{cell}</td>
                                    ))}
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            )}
            {more > 0 && (
                <button type="button" className="command-result-more" onClick={() => setAll(true)}>
                    {tg('command_modal.show_more', { count: more })}
                </button>
            )}
        </div>
    );
}

/**
 * 게임 명령: 모듈·익스텐션이 준 명령을 카드로 보인다 (사용자 11-06 #4, 앱 명령 탭과 같은 규칙).
 * 콘솔로 가는 명령이 있으면 위에 입력 줄을 두고 친 줄을 색으로 나눠 안내하며, 그런 카드는 누르면 입력 줄에 이름을 넣는다.
 * API로 요청하는 명령(rest·module)은 입력이 있으면 창에서 칸을 채워 보낸다
 */
export function CommandsPanel({ api, server, moduleData, running }: ServerPanelProps) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const toast = useApp((s) => s.toast);
    const ask = useApp((s) => s.ask);
    const commands = useCommands(api, server, moduleData, t);
    const [filter, setFilter] = useState('');
    const [open, setOpen] = useState<LocalCommand | null>(null);
    const [values, setValues] = useState<Record<string, unknown>>({});
    const [busy, setBusy] = useState(false);
    const [results, setResults] = useState<CommandResult[]>([]);
    const [line, setLine] = useState('');
    const lineRef = useRef<HTMLInputElement>(null);
    // 콘솔로 가는 명령이 하나라도 있으면 입력 줄을 둔다 (칸 정보가 없으면 평범한 줄이다)
    const hasLine = commands.some(isLineCommand);
    const lineCommands = commands as Parameters<typeof readCommandLine>[1];
    const syntax = readCommandLine(line, lineCommands);

    const q = filter.trim().toLowerCase();
    const shown = commands.filter(
        (c) => !q || c.name.toLowerCase().includes(q) || String(c.label).toLowerCase().includes(q),
    );

    /** 보내고 결과를 남긴다. 위험한 명령은 한 번 더 묻는다. 보냈으면 true */
    const send = async (body: { command: string; args?: Record<string, unknown> }, label: string, danger: boolean) => {
        if (busy) return false; // 응답을 기다리는 동안 같은 명령을 다시 보내지 않는다
        if (danger) {
            const ok = await ask({
                title: t('console.danger_title'),
                message: t('console.danger_message', { command: label }),
                danger: true,
            });
            if (!ok) return false;
        }
        setBusy(true);
        const res = await api.executeCommand(server.id, body);
        setBusy(false);
        const ok = !failed(res);
        setResults((prev) =>
            [
                {
                    id: Date.now(),
                    label,
                    ok,
                    output: ok && res?.output && typeof res.output === 'object' ? res.output : null,
                    text: ok ? resultText(res) : errorText(t, res),
                },
                ...prev,
            ].slice(0, 20),
        );
        if (!ok) toast(errorText(t, res), 'error');
        return ok;
    };

    const run = async (cmd: LocalCommand, given: Record<string, unknown>) => {
        const body = { command: cmd.name, args: commandArgs(cmd.inputs, given) };
        if (await send(body, String(cmd.label), cmd.risk === 'danger')) setOpen(null);
    };

    // 입력 줄: 친 줄 그대로 보낸다 (명령 찾기·인자 나누기·검사는 데몬이 한다. 목록에 없는 명령도 콘솔로 간다)
    const sendLine = async () => {
        const text = line.trim();
        if (!text) return;
        if (await send({ command: text }, text, needsConfirm(lineCommands, text))) setLine('');
    };

    const pick = (cmd: LocalCommand) => {
        if (hasLine && isLineCommand(cmd) && cmd.inputs?.length) {
            setLine(`${cmd.name} `);
            lineRef.current?.focus();
            return;
        }
        if (cmd.inputs?.length) {
            setValues(defaultsOf(cmd));
            setOpen(cmd);
        } else run(cmd, {});
    };

    const missing = open ? missingInputs(open.inputs, values) : [];

    return (
        <div className="panel">
            {!running && <p className="banner banner-info">{tg('shell.commands_offline')}</p>}
            {hasLine && (
                <form
                    className="command-line"
                    onSubmit={(e) => {
                        e.preventDefault();
                        sendLine();
                    }}
                >
                    <div className="console-input">
                        <span className="command-line-box">
                            <CommandLineField
                                state={syntax}
                                inputRef={lineRef}
                                value={line}
                                onChange={(e) => setLine(e.target.value)}
                                onKeyDown={(e) => {
                                    // Tab: 이어 칠 이름이나 고를 값의 첫째로 채운다
                                    if (e.key === 'Tab' && syntax.suggestions) {
                                        e.preventDefault();
                                        setLine(applySuggestion(line, syntax, syntax.suggestions.values[0]));
                                    }
                                }}
                                placeholder={tg('command_modal.command_placeholder')}
                                aria-label={tg('command_modal.command_label')}
                                autoCapitalize="off"
                                autoCorrect="off"
                                autoComplete="off"
                                spellCheck={false}
                                enterKeyHint="send"
                                disabled={!running}
                            />
                        </span>
                        <Button tone="accent" type="submit" busy={busy} disabled={!running || !line.trim()}>
                            {t('console.send')}
                        </Button>
                    </div>
                    <CommandLineHelp
                        text={line}
                        state={syntax}
                        moduleName={server.module}
                        onPick={(v) => {
                            setLine(v);
                            lineRef.current?.focus();
                        }}
                    />
                </form>
            )}
            {commands.length > 6 && (
                <input
                    className="search"
                    type="search"
                    value={filter}
                    placeholder={tg('maintenance.command_search')}
                    aria-label={tg('maintenance.command_search')}
                    onChange={(e) => setFilter(e.target.value)}
                />
            )}
            {shown.length === 0 ? (
                <Empty>{tg('maintenance.command_none')}</Empty>
            ) : (
                <ul className="command-grid">
                    {shown.map((c) => (
                        <li key={c.name}>
                            <button
                                type="button"
                                className={c.risk === 'danger' ? 'command-card danger' : 'command-card'}
                                disabled={!running || busy}
                                onClick={() => pick(c)}
                            >
                                <span className="command-name">
                                    {c.label}
                                    {c.risk === 'danger' && (
                                        <em className="tag tag-danger">{tg('maintenance.command_danger_chip')}</em>
                                    )}
                                </span>
                                {c.description && <span className="command-desc">{c.description}</span>}
                                {(c.inputs?.length ?? 0) > 0 && (
                                    <span className="command-inputs">
                                        {c.inputs?.map((i) => i.label || i.name).join(' · ')}
                                    </span>
                                )}
                            </button>
                        </li>
                    ))}
                </ul>
            )}
            {results.length > 0 && (
                <Section title={t('commands.results')}>
                    <ul className="rows">
                        {results.map((r) => (
                            <li key={r.id} className="row">
                                <div className="row-main">
                                    <div className="row-title">
                                        <Icon name={r.ok ? 'checkCircle' : 'alertCircle'} size={16} /> {r.label}
                                    </div>
                                    {r.output ? (
                                        <CommandOutput output={r.output} />
                                    ) : (
                                        r.text && <pre className="row-sub command-output">{r.text}</pre>
                                    )}
                                </div>
                            </li>
                        ))}
                    </ul>
                </Section>
            )}
            {open && (
                <Sheet title={open.label} onClose={() => setOpen(null)}>
                    <form
                        className="form"
                        onSubmit={(e) => {
                            e.preventDefault();
                            run(open, values);
                        }}
                    >
                        {open.description && <p className="hint">{open.description}</p>}
                        {(open.inputs ?? []).map((input) => (
                            <CommandInput
                                key={input.name}
                                input={input}
                                value={values[input.name] ?? ''}
                                onChange={(v) => setValues((prev) => ({ ...prev, [input.name]: v }))}
                            />
                        ))}
                        <Button
                            tone={open.risk === 'danger' ? 'danger' : 'accent'}
                            type="submit"
                            busy={busy}
                            disabled={missing.length > 0}
                        >
                            {t('commands.run')}
                        </Button>
                    </form>
                </Sheet>
            )}
        </div>
    );
}
