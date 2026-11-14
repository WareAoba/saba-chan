import clsx from 'clsx';
import type { TFunction } from 'i18next';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useInstanceCommands } from '../../hooks/useInstanceCommands';
import type { Server } from '../../stores/useServerStore';
import type { ApiBody, CommandInput as CommandInputDef } from '../../types/daemon';
import { subscribeDaemonEvents } from '../../utils/daemonEvents';
import { safeShowToast } from '../../utils/helpers';
import {
    ACTIONS,
    COMMAND,
    draftProblems,
    fromDraft,
    MAX_EVERY_DAYS,
    MODES,
    offsetText,
    type ScheduledTask,
    type TaskDraft,
    toDraft,
    WARNS,
    WEEKDAYS,
    weekdayLabel,
} from '../../utils/schedule';
import { commandText } from '../../utils/serverView';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { DateField, TimeField } from '../ui/DateTimePicker';
import { SabaNumberInput, SabaToggle } from '../ui/SabaUI';
import { Segmented, StatusChip } from '../ui/StatusChip';
import { failed } from '../../utils/apiResult';
import { SaveBar } from '../ui/SaveBar';
import { Dialog } from '../ui/Dialog';
import { Disclosure } from '../ui/Disclosure';
import { auditReason } from '../../utils/audit';
import { errorText } from './maintenanceText';

/** 모듈·익스텐션 명령 한 개 (입력 정의만 좁혀 읽는다) */
type LocalCommand = ApiBody & { inputs?: CommandInputDef[] };

/** 예약 작업이 한 번 돈 기록 (데몬 `history`의 한 건) */
interface TaskRun {
    at: number;
    result: 'ok' | 'failed' | 'skipped';
    code?: string;
    seconds?: number;
}

const RUN_LOOK = {
    ok: { icon: 'checkCircle', tone: 'ok' },
    failed: { icon: 'xCircle', tone: 'stop' },
    skipped: { icon: 'clock', tone: 'idle' },
} as const;

/** 한 건 → "성공 · 12초", "실패 · 이유". 문구가 없는 이유 코드는 코드 그대로 보인다 */
function runText(run: TaskRun, t: TFunction) {
    const secs = run.seconds ?? 0;
    const took =
        secs >= 60
            ? t('maintenance.run_took_mins', { m: Math.floor(secs / 60), s: secs % 60 })
            : t('maintenance.run_took_secs', { s: secs });
    const reason =
        run.code &&
        (t(`maintenance.run_code_${run.code}`, { defaultValue: '' }) ||
            auditReason({ ok: false, code: run.code }, t) ||
            run.code);
    const detail = run.result === 'ok' ? took : reason;
    return [t(`maintenance.run_${run.result}`), detail].filter(Boolean).join(' · ');
}

/** 예약 탭이 읽어 둔 데몬 응답 (GET /api/instance/:id/schedule) */
export interface ScheduleInfo {
    schedule: ScheduledTask[];
    backupKeep: number;
    backupSupported: boolean;
    announceSupported: boolean;
    saveSupported: boolean;
    nextRuns: number[];
    history: TaskRun[][];
    utcOffset: number | null;
    revision?: string;
}

// 데몬 `commands::find`와 같이 이름은 대소문자를 가리지 않고, 별명으로 저장한 것도 찾는다
const findCommand = (commands: LocalCommand[], name: unknown): LocalCommand | undefined => {
    const want = String(name || '').toLowerCase();
    if (!want) return undefined;
    return (
        commands.find((c: ApiBody) => c.name?.toLowerCase() === want) ||
        commands.find((c) => (c.aliases || []).some((a: unknown) => String(a).toLowerCase() === want))
    );
};
// 키 순서와 상관없이 같은 작업인지 견줄 문자열
const sameKey = (v: unknown): string =>
    v && typeof v === 'object' && !Array.isArray(v)
        ? `{${Object.keys(v)
              .sort()
              .map((k) => `${JSON.stringify(k)}:${sameKey((v as Record<string, unknown>)[k])}`)
              .join(',')}}`
        : JSON.stringify(v);

export function ScheduleCard({
    server,
    info,
    onSaved,
    moduleCommands,
}: {
    server: Server;
    info: ScheduleInfo;
    onSaved: () => void;
    moduleCommands: ApiBody[];
}) {
    const { t, i18n } = useTranslation('gui');
    const base = useRef<string | null>(null);
    const [drafts, setDrafts] = useState<TaskDraft[]>([]);
    const [keep, setKeep] = useState(String(info.backupKeep));
    const [saving, setSaving] = useState(false);
    const [serverErrors, setServerErrors] = useState<string[]>([]);
    // 다른 곳에서 바뀌어 거절됨: 고친 내용은 두고 다시 읽기를 보인다
    const [stale, setStale] = useState(false);
    // 게임 명령 목록을 띄운 작업
    const [picking, setPicking] = useState<string | null>(null);
    // 새로 넣은 위험 명령: 한 번 더 눌러 저장한다
    const [armed, setArmed] = useState(false);
    const commands = localizeCommands(t, server.module, useInstanceCommands(server, moduleCommands) || []);
    // 다음 실행 시각: 작업이 돈 뒤에는 이것만 다시 읽는다 (고치던 내용은 그대로 둔다)
    const [nextRuns, setNextRuns] = useState(info.nextRuns);
    const [history, setHistory] = useState(info.history);
    useEffect(() => {
        setNextRuns(info.nextRuns);
        setHistory(info.history);
    }, [info]);
    const refreshRuns = useCallback(async () => {
        try {
            const res = await window.api.instanceScheduleGet(server.id);
            if (res?.success !== false && res?.revision === info.revision && Array.isArray(res?.next_runs)) {
                setNextRuns(res.next_runs);
                if (Array.isArray(res?.history)) setHistory(res.history);
            }
        } catch {
            // 다음에 탭을 열 때 다시 읽는다
        }
    }, [info.revision, server.id]);
    useEffect(() => {
        const nowSecs = Date.now() / 1000;
        const soonest = Math.min(...nextRuns.filter((s: number) => typeof s === 'number' && s > nowSecs));
        if (!Number.isFinite(soonest)) return undefined;
        // 실행기는 30초마다 본다: 그 뒤에 읽는다
        const wait = Math.min((soonest - nowSecs + 40) * 1000, 2 ** 31 - 1);
        const timer = setTimeout(refreshRuns, wait);
        return () => clearTimeout(timer);
    }, [nextRuns, refreshRuns]);
    // 이력은 작업이 끝난 뒤에 남는다: 오래 걸린 백업·재시작은 끝났다는 이벤트에 다시 읽는다
    useEffect(
        () =>
            subscribeDaemonEvents((ev) => {
                if ((ev.type === 'schedule.ran' || ev.type === 'schedule.failed') && ev.data.instanceId === server.id) {
                    refreshRuns();
                }
            }),
        [refreshRuns, server.id],
    );

    useEffect(() => {
        const next = info.schedule.map(toDraft);
        setDrafts(next);
        setKeep(String(info.backupKeep));
        setServerErrors([]);
        setStale(false);
        setArmed(false);
        // 다시 읽은 작업은 key가 새로 생긴다: 고르던 명령 목록은 닫는다
        setPicking(null);
        base.current = JSON.stringify({ d: next.map(({ key, ...r }) => r), k: String(info.backupKeep) });
    }, [info]);

    const snapshot = JSON.stringify({ d: drafts.map(({ key, ...r }) => r), k: keep });
    const dirty = base.current !== null && snapshot !== base.current;

    const actions = ACTIONS.filter(
        (a) =>
            (a !== 'backup' || info.backupSupported) &&
            (a !== 'announce' || info.announceSupported) &&
            (a !== 'save' || info.saveSupported),
    );
    const keepNum = Number(keep);
    const keepBad = info.backupSupported && (!Number.isInteger(keepNum) || keepNum < 1);
    const problems = drafts.map((d) => draftProblems(d, t, findCommand(commands, d.command)));
    const blocked = keepBad || problems.some((p) => p.length > 0);
    const toTask = (d: TaskDraft) =>
        fromDraft(d, { announce: info.announceSupported, inputs: findCommand(commands, d.command)?.inputs });
    const saved = info.schedule.map(sameKey);

    // 저장된 그대로인 켜진 작업만 다음 실행을 보인다 (고친 작업은 저장해야 정해진다).
    // 시각이 지났거나 다시 돌 일이 없는 날짜 한 번 작업은 끝났다고 보인다
    const nextRunOf = (d: TaskDraft): { at?: number; done?: boolean } | null => {
        const i = saved.indexOf(sameKey(toTask(d)));
        if (i < 0) return null;
        const at = nextRuns[i];
        if (typeof at === 'number' && at > Date.now() / 1000) return { at };
        return d.mode === 'date' ? { done: true } : null;
    };
    // 저장된 그대로인 작업의 실행 기록 (고친 작업은 새 작업이라 기록이 없다)
    const runsOf = (d: TaskDraft): TaskRun[] | null => {
        const i = saved.indexOf(sameKey(toTask(d)));
        return i < 0 ? null : history[i] || [];
    };
    const fmtRun = (secs: number) =>
        new Date(secs * 1000).toLocaleString(i18n.language, {
            month: 'numeric',
            day: 'numeric',
            weekday: 'short',
            hour: '2-digit',
            minute: '2-digit',
        });
    // 서버 PC와 보는 곳의 시간대가 다르면 시각이 어느 쪽 기준인지 알린다
    const otherZone = info.utcOffset !== null && info.utcOffset !== -new Date().getTimezoneOffset();
    // 이번에 새로 넣거나 바꾼 켜진 위험 명령
    const newDanger = drafts.some(
        (d) =>
            d.enabled &&
            d.action === COMMAND &&
            findCommand(commands, d.command)?.risk === 'danger' &&
            !saved.includes(sameKey(toTask(d))),
    );

    const patch = (key: string, change: Partial<TaskDraft>) => {
        setArmed(false);
        setDrafts((ds) => ds.map((d) => (d.key === key ? { ...d, ...change } : d)));
    };

    const save = async () => {
        if (newDanger && !armed) {
            setArmed(true);
            return;
        }
        setArmed(false);
        setSaving(true);
        setServerErrors([]);
        try {
            const body: ApiBody = { schedule: drafts.map(toTask) };
            if (info.backupSupported) body.backup_keep = keepNum;
            if (info.revision) body.revision = info.revision;
            const res = await window.api.instanceScheduleSave(server.id, body);
            if (failed(res)) {
                setStale(res.error_code === 'schedule_changed');
                setServerErrors(
                    Array.isArray(res.details) && res.details.length ? res.details : [errorText(res, 'error', t)],
                );
                return;
            }
            safeShowToast(t('maintenance.schedule_saved'), 'success', 3000);
            // 이미 지난 날의 한 번 작업은 저장은 됐지만 돌지 않는다 (데몬 warnings, task는 0부터)
            for (const w of Array.isArray(res?.warnings) ? res.warnings : []) {
                if (w?.code === 'date_passed' && Number.isInteger(w.task)) {
                    safeShowToast(t('maintenance.schedule_date_passed', { n: w.task + 1 }), 'warning', 8000);
                }
            }
            onSaved();
        } catch (err) {
            setServerErrors([(err as Error).message || String(err)]);
        } finally {
            setSaving(false);
        }
    };

    const pickingDraft = drafts.find((d) => d.key === picking);

    return (
        <section className="dc-card" aria-label={t('maintenance.schedule_title')}>
            <div className="dc-card-head">
                <h2>{t('maintenance.schedule_title')}</h2>
                <button data-spot="schedule.add"
                    type="button"
                    className="btn btn-ghost btn-sm"
                    onClick={() =>
                        setDrafts((ds) => [
                            ...ds,
                            toDraft({ action: actions.includes('backup') ? 'backup' : 'restart' }),
                        ])
                    }
                >
                    <Icon name="plus" size="sm" /> {t('maintenance.task_add')}
                </button>
            </div>
            <p className="dc-card-desc">{t('maintenance.schedule_desc')}</p>
            {otherZone && (
                <p className="muted">
                    <Icon name="info" size="xs" /> {t('maintenance.server_clock', { zone: offsetText(info.utcOffset as number) })}
                </p>
            )}

            {drafts.length === 0 ? (
                <p className="muted">{t('maintenance.schedule_empty')}</p>
            ) : (
                <ul className="mt-tasks">
                    {drafts.map((d, i) => {
                        const shown =
                            d.action === COMMAND || actions.includes(d.action) ? actions : [...actions, d.action];
                        const items = shown.map((a) => ({
                            id: a,
                            label: t(`maintenance.action_${a}`, { defaultValue: a }),
                        }));
                        const canWarn = WARNS.includes(d.action) && info.announceSupported;
                        const command = d.action === COMMAND ? findCommand(commands, d.command) : null;
                        const next = d.enabled ? nextRunOf(d) : null;
                        const runs = runsOf(d);
                        return (
                            <li key={d.key} data-spot="schedule.task" className={clsx('mt-task', { off: !d.enabled })}>
                                <div className="mt-task-head">
                                    <Segmented
                                        items={items}
                                        value={d.action}
                                        onChange={(a) => patch(d.key, { action: a })}
                                        ariaLabel={t('maintenance.task_aria', { n: i + 1 })}
                                    />
                                    <button
                                        type="button"
                                        className={clsx(
                                            'btn btn-sm',
                                            d.action === COMMAND ? 'btn-primary' : 'btn-ghost',
                                        )}
                                        onClick={() => setPicking(d.key)}
                                    >
                                        <Icon name="terminal" size="sm" />{' '}
                                        {d.action === COMMAND && d.command
                                            ? command?.label || d.command
                                            : t('maintenance.action_command')}
                                    </button>
                                    <SabaToggle
                                        checked={d.enabled}
                                        aria-label={t('maintenance.task_enabled')}
                                        title={t('maintenance.task_enabled')}
                                        onChange={(v) => patch(d.key, { enabled: v })}
                                    />
                                    <button
                                        type="button"
                                        className="icon-btn"
                                        data-tooltip={t('maintenance.task_remove')}
                                        aria-label={t('maintenance.task_remove')}
                                        onClick={() => setDrafts((ds) => ds.filter((x) => x.key !== d.key))}
                                    >
                                        <Icon name="trash" size="sm" />
                                    </button>
                                </div>
                                <div className="mt-task-fields">
                                    <label className="mt-field">
                                        <span>{t('maintenance.when')}</span>
                                        <span className="mt-when">
                                            <CustomDropdown
                                                value={d.mode}
                                                ariaLabel={t('maintenance.when')}
                                                onChange={(v) => patch(d.key, { mode: v })}
                                                options={MODES.map((m) => ({
                                                    value: m,
                                                    label: t(`maintenance.mode_${m}`),
                                                }))}
                                            />
                                            {d.mode === 'every' && (
                                                <span className="mt-inline">
                                                    <SabaNumberInput
                                                        className="input mt-num"
                                                        min="1"
                                                        value={d.every}
                                                        aria-label={t('maintenance.every_minutes')}
                                                        onChange={(e) => patch(d.key, { every: e.target.value })}
                                                    />
                                                    <span className="muted">{t('maintenance.minutes')}</span>
                                                </span>
                                            )}
                                            {d.mode === 'days' && (
                                                <span className="mt-inline">
                                                    <SabaNumberInput
                                                        className="input mt-num"
                                                        min="1"
                                                        max={MAX_EVERY_DAYS}
                                                        value={d.everyDays}
                                                        aria-label={t('maintenance.every_days')}
                                                        onChange={(e) => patch(d.key, { everyDays: e.target.value })}
                                                    />
                                                    <span className="muted">{t('maintenance.days')}</span>
                                                </span>
                                            )}
                                            {d.mode === 'date' && (
                                                <DateField
                                                    className="mt-date"
                                                    value={d.date}
                                                    ariaLabel={t('maintenance.on_date')}
                                                    onChange={(v) => patch(d.key, { date: v })}
                                                />
                                            )}
                                            {d.mode !== 'every' && (
                                                <TimeField
                                                    className="mt-time"
                                                    value={d.at}
                                                    ariaLabel={t('maintenance.at_time')}
                                                    onChange={(v) => patch(d.key, { at: v })}
                                                />
                                            )}
                                        </span>
                                    </label>
                                    {d.mode === 'days' && (
                                        <label className="mt-field">
                                            <span>{t('maintenance.start_date')}</span>
                                            <DateField
                                                className="mt-date"
                                                value={d.startDate}
                                                ariaLabel={t('maintenance.start_date')}
                                                title={t('maintenance.start_date_hint')}
                                                onChange={(v) => patch(d.key, { startDate: v })}
                                            />
                                        </label>
                                    )}
                                    {d.mode === 'weekdays' && (
                                        <fieldset className="mt-field mt-fieldset">
                                            <legend>{t('maintenance.weekdays')}</legend>
                                            <span className="mt-weekdays">
                                                {WEEKDAYS.map((w, wi) => {
                                                    const on = d.weekdays.includes(w);
                                                    return (
                                                        <button
                                                            key={w}
                                                            type="button"
                                                            aria-pressed={on}
                                                            className={clsx('mt-weekday', { on })}
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
                                    {canWarn && (
                                        <label className="mt-field">
                                            <span>{t('maintenance.warn_minutes')}</span>
                                            <input
                                                className="input"
                                                value={d.warn}
                                                placeholder="10, 1"
                                                onChange={(e) => patch(d.key, { warn: e.target.value })}
                                            />
                                        </label>
                                    )}
                                    {d.action === 'announce' && (
                                        <label className="mt-field mt-field-wide">
                                            <span>{t('maintenance.message')}</span>
                                            <input
                                                className="input"
                                                value={d.message}
                                                placeholder={t('maintenance.message_placeholder')}
                                                onChange={(e) => patch(d.key, { message: e.target.value })}
                                            />
                                        </label>
                                    )}
                                    {command?.inputs?.map((input) => (
                                        <CommandInput
                                            key={input.name}
                                            input={input}
                                            value={d.args[input.name] ?? ''}
                                            onChange={(v) => patch(d.key, { args: { ...d.args, [input.name]: v } })}
                                        />
                                    ))}
                                </div>
                                {command?.risk === 'danger' && (
                                    <p className="dc-field-error">
                                        <Icon name="alertTriangle" size="xs" /> {t('maintenance.command_danger')}
                                    </p>
                                )}
                                {next?.at && (
                                    <p className="muted mt-next">
                                        {t('maintenance.next_run', { time: fmtRun(next.at) })}
                                    </p>
                                )}
                                {next?.done && <p className="muted mt-next">{t('maintenance.date_done')}</p>}
                                {runs?.length === 0 && (
                                    <p className="muted mt-next mt-last">
                                        <Icon name="clock" size="xs" /> {t('maintenance.last_run_none')}
                                    </p>
                                )}
                                {runs && runs.length > 0 && (
                                    <Disclosure
                                        className="mt-runs"
                                        summary={
                                            <span className={`mt-last tone-${RUN_LOOK[runs[0].result]?.tone}`}>
                                                <Icon name={RUN_LOOK[runs[0].result]?.icon || 'clock'} size="xs" />
                                                {t('maintenance.last_run', {
                                                    time: fmtRun(runs[0].at),
                                                    result: runText(runs[0], t),
                                                })}
                                            </span>
                                        }
                                    >
                                        <ul className="dc-list">
                                            {runs.map((run) => (
                                                <li key={run.at} className="dc-list-row">
                                                    <span className={`mt-last tone-${RUN_LOOK[run.result]?.tone}`}>
                                                        <Icon name={RUN_LOOK[run.result]?.icon || 'clock'} size="sm" />
                                                    </span>
                                                    <div className="dc-list-text">
                                                        <div className="dc-list-name">{fmtRun(run.at)}</div>
                                                        <div className="dc-list-sub">{runText(run, t)}</div>
                                                    </div>
                                                </li>
                                            ))}
                                        </ul>
                                    </Disclosure>
                                )}
                                {problems[i].length > 0 && (
                                    <p className="dc-field-error">
                                        <Icon name="alertTriangle" size="xs" /> {problems[i].join(' · ')}
                                    </p>
                                )}
                            </li>
                        );
                    })}
                </ul>
            )}

            {info.backupSupported && (
                <label className="mt-field mt-keep">
                    <span>{t('maintenance.backup_keep')}</span>
                    <span className="mt-inline">
                        <SabaNumberInput
                            className="input mt-num"
                            min="1"
                            value={keep}
                            onChange={(e) => setKeep(e.target.value)}
                        />
                        <span className="muted">{t('maintenance.backup_keep_hint')}</span>
                    </span>
                </label>
            )}

            {serverErrors.length > 0 && (
                <div className="notice tone-warn" role="alert">
                    <Icon name="alertTriangle" size="sm" />
                    <span className="mt-pre">{serverErrors.join('\n')}</span>
                    {stale && (
                        <button type="button" className="btn btn-ghost btn-sm" onClick={onSaved}>
                            {t('maintenance.reload')}
                        </button>
                    )}
                </div>
            )}

            <SaveBar
                dirty={dirty}
                saving={saving}
                blocked={blocked}
                danger={armed}
                message={armed ? t('maintenance.danger_confirm') : undefined}
                saveLabel={armed ? t('maintenance.danger_save') : undefined}
                saveSpot="schedule.save"
                onSave={save}
                onRevert={() => {
                    setDrafts(info.schedule.map(toDraft));
                    setKeep(String(info.backupKeep));
                    setServerErrors([]);
                    setArmed(false);
                }}
            />
            {pickingDraft && (
                <CommandPicker
                    commands={commands}
                    current={pickingDraft.action === COMMAND ? pickingDraft.command : ''}
                    onPick={(c) => {
                        const keepArgs = c.name === pickingDraft.command ? pickingDraft.args : {};
                        const defaults = Object.fromEntries(
                            (c.inputs || [])
                                .filter((input) => input.default != null && keepArgs[input.name] == null)
                                .map((input) => [input.name, String(input.default)]),
                        );
                        patch(pickingDraft.key, {
                            action: COMMAND,
                            command: c.name,
                            args: { ...defaults, ...keepArgs },
                        });
                        setPicking(null);
                    }}
                    onClose={() => setPicking(null)}
                />
            )}
        </section>
    );
}

/** 명령과 입력에 모듈 locales 문구를 입힌다 (`mod_<모듈>:commands.<이름>`) */
function localizeCommands(t: TFunction, moduleName: string, list: ApiBody[]): LocalCommand[] {
    return list.map((c) => {
        const ns = `mod_${moduleName}:commands.${c.name}.inputs`;
        return {
            ...c,
            ...commandText(t, moduleName, c as Parameters<typeof commandText>[2]),
            inputs: c.inputs?.map((input: CommandInputDef) => ({
                ...input,
                label: t(`${ns}.${input.name}.label`, { defaultValue: input.label || input.name }),
                placeholder: t(`${ns}.${input.name}.placeholder`, { defaultValue: input.placeholder || '' }),
            })),
        };
    });
}

/** 게임 명령 입력 하나 (module.toml `[[commands.fields.inputs]]`) */
function CommandInput({
    input,
    value,
    onChange,
}: {
    input: CommandInputDef;
    value: string;
    onChange: (value: string) => void;
}) {
    const label = `${input.label || input.name}${input.required ? ' *' : ''}`;
    if (Array.isArray(input.options) && input.options.length) {
        return (
            <div className="mt-field">
                <span>{label}</span>
                <CustomDropdown
                    value={value}
                    ariaLabel={label}
                    onChange={onChange}
                    options={[
                        ...(input.required ? [] : [{ value: '', label: '—' }]),
                        ...input.options.map((o: unknown) => ({ value: String(o), label: String(o) })),
                    ]}
                />
            </div>
        );
    }
    return (
        <label className="mt-field">
            <span>{label}</span>
            <input
                className={clsx('input', input.type === 'number' && 'mt-num')}
                type={input.type === 'number' ? 'number' : 'text'}
                min={input.min}
                max={input.max}
                value={value}
                placeholder={input.placeholder || ''}
                onChange={(e) => onChange(e.target.value)}
            />
        </label>
    );
}

/** 예약할 게임 명령을 고르는 창 (모듈·익스텐션이 주는 명령) */
function CommandPicker({
    commands,
    current,
    onPick,
    onClose,
}: {
    commands: LocalCommand[];
    current: string;
    onPick: (command: LocalCommand) => void;
    onClose: () => void;
}) {
    const { t } = useTranslation('gui');
    const [filter, setFilter] = useState('');
    const q = filter.trim().toLowerCase();
    const list = commands.filter(
        (c) =>
            !q ||
            c.name.toLowerCase().includes(q) ||
            String(c.label || '')
                .toLowerCase()
                .includes(q),
    );
    return (
        <Dialog
            open
            onOpenChange={(o) => !o && onClose()}
            size="md"
            className="mt-picker"
            title={t('maintenance.command_pick')}
            footer={
                <button type="button" className="btn btn-ghost" onClick={onClose}>
                    {t('maintenance.close')}
                </button>
            }
        >
            <div className="mt-picker-body">
                <input
                    className="input"
                    value={filter}
                    placeholder={t('maintenance.command_search')}
                    aria-label={t('maintenance.command_search')}
                    onChange={(e) => setFilter(e.target.value)}
                />
                {list.length === 0 ? (
                    <p className="muted">{t('maintenance.command_none')}</p>
                ) : (
                    <ul className="saba-pick-list">
                        {list.map((c) => (
                            <li key={c.name}>
                                <button
                                    type="button"
                                    className={clsx('saba-pick-item', { selected: c.name === current })}
                                    onClick={() => onPick(c)}
                                >
                                    <span className="mt-picker-name">
                                        {c.label || c.name}
                                        {c.label && c.label !== c.name && <code>{c.name}</code>}
                                    </span>
                                    {c.description && <span className="muted">{c.description}</span>}
                                    {c.risk === 'danger' && (
                                        <StatusChip tone="stop" label={t('maintenance.command_danger_chip')} />
                                    )}
                                </button>
                            </li>
                        ))}
                    </ul>
                )}
            </div>
        </Dialog>
    );
}
