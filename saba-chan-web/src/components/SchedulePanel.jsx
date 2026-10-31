import {
    ACTIONS,
    COMMAND,
    draftProblems,
    fromDraft,
    MAX_EVERY_DAYS,
    MODES,
    offsetText,
    toDraft,
    WARNS,
    WEEKDAYS,
    weekdayLabel,
} from '@gui/utils/schedule';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import { CommandInput } from './CommandsPanel';
import { useCommands } from './commands';
import { Dropdown } from './Dropdown';
import { Icon } from './icons';
import { Button, Empty, errorText, failed, Loading, Section } from './ui';

const strip = (drafts, keep) => JSON.stringify({ d: drafts.map(({ key, ...r }) => r), k: keep });
const findCommand = (commands, name) => commands.find((c) => c.name === name);
// 저장된 작업과 같은지 (키 순서와 무관하게 비교)
const sameKey = (v) =>
    v && typeof v === 'object'
        ? Array.isArray(v)
            ? `[${v.map(sameKey).join(',')}]`
            : `{${Object.keys(v)
                  .sort()
                  .map((k) => `${JSON.stringify(k)}:${sameKey(v[k])}`)
                  .join(',')}}`
        : JSON.stringify(v);

/** 예약 작업 (GUI 백업·예약 탭과 같은 규칙: @gui/utils/schedule). 시각은 서버 PC의 시계다 */
export function SchedulePanel({ api, server, moduleData }) {
    const { t, i18n } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const toast = useApp((s) => s.toast);
    const commands = useCommands(api, server, moduleData, t);
    const [info, setInfo] = useState(null);
    const [drafts, setDrafts] = useState([]);
    const [keep, setKeep] = useState('10');
    const [saving, setSaving] = useState(false);
    const [armed, setArmed] = useState(false);
    const [errors, setErrors] = useState([]);
    const [stale, setStale] = useState(false);
    const base = useRef(null);

    const load = useCallback(async () => {
        const res = await api.instanceScheduleGet(server.id);
        if (failed(res)) {
            setInfo({ error: errorText(t, res) });
            return;
        }
        const next = {
            schedule: Array.isArray(res.schedule) ? res.schedule : [],
            backupKeep: res.backup_keep ?? 10,
            backupSupported: !!res.backup_supported,
            announceSupported: !!res.announce_supported,
            saveSupported: !!res.save_supported,
            nextRuns: Array.isArray(res.next_runs) ? res.next_runs : [],
            utcOffset: typeof res.utc_offset_minutes === 'number' ? res.utc_offset_minutes : null,
            revision: res.revision,
        };
        const d = next.schedule.map(toDraft);
        setInfo(next);
        setDrafts(d);
        setKeep(String(next.backupKeep));
        setErrors([]);
        setStale(false);
        setArmed(false);
        base.current = strip(d, String(next.backupKeep));
    }, [api, server.id, t]);

    useEffect(() => {
        load();
    }, [load]);

    if (!info) return <Loading />;
    if (info.error)
        return (
            <Section title={tg('maintenance.schedule_title')}>
                <p className="banner banner-error">{info.error}</p>
            </Section>
        );

    const actions = ACTIONS.filter(
        (a) =>
            (a !== 'backup' || info.backupSupported) &&
            (a !== 'announce' || info.announceSupported) &&
            (a !== 'save' || info.saveSupported),
    );
    const dirty = base.current !== null && strip(drafts, keep) !== base.current;
    const keepNum = Number(keep);
    const keepBad = info.backupSupported && (!Number.isInteger(keepNum) || keepNum < 1);
    const problems = drafts.map((d) => draftProblems(d, tg, findCommand(commands, d.command)));
    const blocked = keepBad || problems.some((p) => p.length > 0);
    const toTask = (d) =>
        fromDraft(d, { announce: info.announceSupported, inputs: findCommand(commands, d.command)?.inputs });
    const saved = info.schedule.map(sameKey);
    const newDanger = drafts.some(
        (d) =>
            d.enabled &&
            d.action === COMMAND &&
            findCommand(commands, d.command)?.risk === 'danger' &&
            !saved.includes(sameKey(toTask(d))),
    );
    const otherZone = info.utcOffset !== null && info.utcOffset !== -new Date().getTimezoneOffset();
    const fmtRun = (secs) =>
        new Date(secs * 1000).toLocaleString(i18n.language, {
            month: 'numeric',
            day: 'numeric',
            weekday: 'short',
            hour: '2-digit',
            minute: '2-digit',
        });
    const nextRunOf = (d) => {
        const i = saved.indexOf(sameKey(toTask(d)));
        if (i < 0) return null;
        const at = info.nextRuns[i];
        if (typeof at === 'number' && at > Date.now() / 1000) return { at };
        return d.mode === 'date' ? { done: true } : null;
    };

    const patch = (key, change) => {
        setArmed(false);
        setDrafts((ds) => ds.map((d) => (d.key === key ? { ...d, ...change } : d)));
    };

    const pickAction = (d, value) => {
        if (!value.startsWith('cmd:')) return patch(d.key, { action: value });
        const name = value.slice(4);
        const cmd = findCommand(commands, name);
        const keepArgs = name === d.command ? d.args : {};
        const defaults = Object.fromEntries(
            (cmd?.inputs || [])
                .filter((i) => i.default != null && keepArgs[i.name] == null)
                .map((i) => [i.name, String(i.default)]),
        );
        patch(d.key, { action: COMMAND, command: name, args: { ...defaults, ...keepArgs } });
    };

    const save = async () => {
        if (newDanger && !armed) return setArmed(true);
        setArmed(false);
        setSaving(true);
        setErrors([]);
        const body = { schedule: drafts.map(toTask) };
        if (info.backupSupported) body.backup_keep = keepNum;
        if (info.revision) body.revision = info.revision;
        const res = await api.instanceScheduleSave(server.id, body);
        setSaving(false);
        if (failed(res)) {
            setStale(res?.error_code === 'schedule_changed');
            setErrors(Array.isArray(res?.details) && res.details.length ? res.details : [errorText(t, res)]);
            return;
        }
        toast(tg('maintenance.schedule_saved'), 'ok');
        for (const w of Array.isArray(res?.warnings) ? res.warnings : []) {
            if (w?.code === 'date_passed' && Number.isInteger(w.task))
                toast(tg('maintenance.schedule_date_passed', { n: w.task + 1 }), 'info');
        }
        load();
    };

    const actionOptions = (d) => [
        ...(actions.includes(d.action) || d.action === COMMAND ? actions : [...actions, d.action]).map((a) => ({
            value: a,
            label: tg(`maintenance.action_${a}`, { defaultValue: a }),
        })),
        ...commands.map((c) => ({ value: `cmd:${c.name}`, label: `${tg('maintenance.action_command')} ${c.label}` })),
    ];

    return (
        <Section
            title={tg('maintenance.schedule_title')}
            action={
                <Button
                    onClick={() =>
                        setDrafts((ds) => [
                            ...ds,
                            toDraft({ action: actions.includes('backup') ? 'backup' : 'restart' }),
                        ])
                    }
                >
                    <Icon name="plus" size={16} /> {tg('maintenance.task_add')}
                </Button>
            }
        >
            <p className="hint">{tg('maintenance.schedule_desc')}</p>
            {otherZone && (
                <p className="hint">{tg('maintenance.server_clock', { zone: offsetText(info.utcOffset) })}</p>
            )}
            {drafts.length === 0 ? (
                <Empty>{tg('maintenance.schedule_empty')}</Empty>
            ) : (
                <ul className="tasks">
                    {drafts.map((d, i) => {
                        const command = d.action === COMMAND ? findCommand(commands, d.command) : null;
                        const next = d.enabled ? nextRunOf(d) : null;
                        return (
                            <li key={d.key} className={d.enabled ? 'task' : 'task off'}>
                                <div className="task-head">
                                    <Dropdown
                                        value={d.action === COMMAND ? `cmd:${d.command}` : d.action}
                                        onChange={(v) => pickAction(d, v)}
                                        ariaLabel={tg('maintenance.task_aria', { n: i + 1 })}
                                        options={actionOptions(d)}
                                    />
                                    <input
                                        type="checkbox"
                                        className="switch"
                                        checked={d.enabled}
                                        aria-label={tg('maintenance.task_enabled')}
                                        onChange={(e) => patch(d.key, { enabled: e.target.checked })}
                                    />
                                    <button
                                        type="button"
                                        className="icon-btn"
                                        aria-label={tg('maintenance.task_remove')}
                                        onClick={() => setDrafts((ds) => ds.filter((x) => x.key !== d.key))}
                                    >
                                        <Icon name="trash" size={16} />
                                    </button>
                                </div>
                                <div className="task-fields">
                                    <div className="field">
                                        <span>{tg('maintenance.when')}</span>
                                        <Dropdown
                                            value={d.mode}
                                            ariaLabel={tg('maintenance.when')}
                                            onChange={(v) => patch(d.key, { mode: v })}
                                            options={MODES.map((m) => ({
                                                value: m,
                                                label: tg(`maintenance.mode_${m}`),
                                            }))}
                                        />
                                    </div>
                                    {d.mode === 'every' && (
                                        <label className="field">
                                            <span>{tg('maintenance.every_minutes')}</span>
                                            <input
                                                type="number"
                                                min="1"
                                                inputMode="numeric"
                                                value={d.every}
                                                onChange={(e) => patch(d.key, { every: e.target.value })}
                                            />
                                        </label>
                                    )}
                                    {d.mode === 'days' && (
                                        <>
                                            <label className="field">
                                                <span>{tg('maintenance.every_days')}</span>
                                                <input
                                                    type="number"
                                                    min="1"
                                                    max={MAX_EVERY_DAYS}
                                                    inputMode="numeric"
                                                    value={d.everyDays}
                                                    onChange={(e) => patch(d.key, { everyDays: e.target.value })}
                                                />
                                            </label>
                                            <label className="field">
                                                <span>{tg('maintenance.start_date')}</span>
                                                <input
                                                    type="date"
                                                    value={d.startDate}
                                                    onChange={(e) => patch(d.key, { startDate: e.target.value })}
                                                />
                                                <small className="hint">{tg('maintenance.start_date_hint')}</small>
                                            </label>
                                        </>
                                    )}
                                    {d.mode === 'date' && (
                                        <label className="field">
                                            <span>{tg('maintenance.on_date')}</span>
                                            <input
                                                type="date"
                                                value={d.date}
                                                onChange={(e) => patch(d.key, { date: e.target.value })}
                                            />
                                        </label>
                                    )}
                                    {d.mode !== 'every' && (
                                        <label className="field">
                                            <span>{tg('maintenance.at_time')}</span>
                                            <input
                                                type="time"
                                                value={d.at}
                                                onChange={(e) => patch(d.key, { at: e.target.value })}
                                            />
                                        </label>
                                    )}
                                    {d.mode === 'weekdays' && (
                                        <fieldset className="field weekdays">
                                            <legend>{tg('maintenance.weekdays')}</legend>
                                            <span className="chips">
                                                {WEEKDAYS.map((w, wi) => {
                                                    const on = d.weekdays.includes(w);
                                                    return (
                                                        <button
                                                            key={w}
                                                            type="button"
                                                            aria-pressed={on}
                                                            className={on ? 'chip on' : 'chip'}
                                                            onClick={() =>
                                                                patch(d.key, {
                                                                    weekdays: on
                                                                        ? d.weekdays.filter((x) => x !== w)
                                                                        : [...d.weekdays, w],
                                                                })
                                                            }
                                                        >
                                                            {weekdayLabel(i18n.language, wi)}
                                                        </button>
                                                    );
                                                })}
                                            </span>
                                        </fieldset>
                                    )}
                                    {WARNS.includes(d.action) && info.announceSupported && (
                                        <label className="field">
                                            <span>{tg('maintenance.warn_minutes')}</span>
                                            <input
                                                value={d.warn}
                                                placeholder="10, 1"
                                                onChange={(e) => patch(d.key, { warn: e.target.value })}
                                            />
                                        </label>
                                    )}
                                    {d.action === 'announce' && (
                                        <label className="field field-wide">
                                            <span>{tg('maintenance.message')}</span>
                                            <input
                                                value={d.message}
                                                placeholder={tg('maintenance.message_placeholder')}
                                                onChange={(e) => patch(d.key, { message: e.target.value })}
                                            />
                                        </label>
                                    )}
                                    {command?.inputs?.map((input) => (
                                        <CommandInput
                                            key={input.name}
                                            input={input}
                                            value={d.args[input.name] ?? ''}
                                            onChange={(v) =>
                                                patch(d.key, { args: { ...d.args, [input.name]: String(v) } })
                                            }
                                        />
                                    ))}
                                </div>
                                {command?.risk === 'danger' && (
                                    <p className="field-error">{tg('maintenance.command_danger')}</p>
                                )}
                                {next?.at && (
                                    <p className="hint">{tg('maintenance.next_run', { time: fmtRun(next.at) })}</p>
                                )}
                                {next?.done && <p className="hint">{tg('maintenance.date_done')}</p>}
                                {problems[i].length > 0 && <p className="field-error">{problems[i].join(' · ')}</p>}
                            </li>
                        );
                    })}
                </ul>
            )}
            {info.backupSupported && (
                <label className="field">
                    <span>{tg('maintenance.backup_keep')}</span>
                    <input
                        type="number"
                        min="1"
                        inputMode="numeric"
                        value={keep}
                        onChange={(e) => setKeep(e.target.value)}
                    />
                    <small className="hint">{tg('maintenance.backup_keep_hint')}</small>
                </label>
            )}
            {errors.length > 0 && (
                <div className="banner banner-error" role="alert">
                    <p className="pre">{errors.join('\n')}</p>
                    {stale && <Button onClick={load}>{tg('maintenance.reload')}</Button>}
                </div>
            )}
            <div className="savebar">
                {armed && <span className="hint">{tg('maintenance.danger_confirm')}</span>}
                {dirty && (
                    <Button disabled={saving} onClick={load}>
                        {tg('discord.revert')}
                    </Button>
                )}
                <Button tone={armed ? 'danger' : 'accent'} busy={saving} disabled={!dirty || blocked} onClick={save}>
                    {armed ? tg('maintenance.danger_save') : tg('discord.save')}
                </Button>
            </div>
        </Section>
    );
}
