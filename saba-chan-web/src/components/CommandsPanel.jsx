import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import { commandArgs, missingInputs, useCommands } from './commands';
import { Dropdown } from './Dropdown';
import { Icon } from './icons';
import { Button, Empty, errorText, failed, Section, Sheet } from './ui';

/** 명령 입력 하나 (module.toml `[[commands.fields.inputs]]`) */
export function CommandInput({ input, value, onChange }) {
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

const defaultsOf = (cmd) =>
    Object.fromEntries((cmd.inputs || []).filter((i) => i.default != null).map((i) => [i.name, i.default]));

/** 결과 글자: 게임의 실제 응답(`data.response`, REST는 `data`)을 전송 안내(`message`)보다 먼저 보인다 */
export const resultText = (res) => {
    const data = res?.data;
    const v = data?.response ?? data ?? res?.message ?? res?.text ?? res?.output ?? res?.result;
    if (v == null || v === '') return '';
    return typeof v === 'string' ? v : JSON.stringify(v);
};

/** 게임 명령: 모듈·익스텐션이 준 명령을 카드로 보이고, 입력이 있으면 창에서 채워 보낸다 */
export function CommandsPanel({ api, server, moduleData, running }) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const toast = useApp((s) => s.toast);
    const ask = useApp((s) => s.ask);
    const commands = useCommands(api, server, moduleData, t);
    const [filter, setFilter] = useState('');
    const [open, setOpen] = useState(null);
    const [values, setValues] = useState({});
    const [busy, setBusy] = useState(false);
    const [results, setResults] = useState([]);

    const q = filter.trim().toLowerCase();
    const shown = commands.filter(
        (c) => !q || c.name.toLowerCase().includes(q) || String(c.label).toLowerCase().includes(q),
    );

    const run = async (cmd, given) => {
        if (cmd.risk === 'danger') {
            const ok = await ask({
                title: t('console.danger_title'),
                message: t('console.danger_message', { command: cmd.label }),
                danger: true,
            });
            if (!ok) return;
        }
        setBusy(true);
        const res = await api.executeCommand(server.id, { command: cmd.name, args: commandArgs(cmd.inputs, given) });
        setBusy(false);
        const ok = !failed(res);
        setResults((prev) =>
            [{ id: Date.now(), label: cmd.label, ok, text: ok ? resultText(res) : errorText(t, res) }, ...prev].slice(
                0,
                20,
            ),
        );
        if (ok) setOpen(null);
        else toast(errorText(t, res), 'error');
    };

    const pick = (cmd) => {
        if (cmd.inputs?.length) {
            setValues(defaultsOf(cmd));
            setOpen(cmd);
        } else run(cmd, {});
    };

    const missing = open ? missingInputs(open.inputs, values) : [];

    return (
        <div className="panel">
            {!running && <p className="banner banner-info">{tg('shell.commands_offline')}</p>}
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
                                {c.inputs?.length > 0 && (
                                    <span className="command-inputs">
                                        {c.inputs.map((i) => i.label || i.name).join(' · ')}
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
                                    {r.text && <pre className="row-sub command-output">{r.text}</pre>}
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
                        {open.inputs.map((input) => (
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
