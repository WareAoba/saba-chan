import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useInstanceCommands } from '../../hooks/useInstanceCommands';
import { safeShowToast } from '../../utils/helpers';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaFail } from '../ui/SabaArt';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { Segmented, StatusChip } from '../ui/StatusChip';

const KIND_TONE = {
    manual: 'info',
    scheduled: 'idle',
    'pre-update': 'warn',
    'pre-restore': 'warn',
    snapshot: 'info',
    'game-snapshot': 'idle',
};
// 게임이 스스로 만든 자동 백업 (데몬 `game-snapshot:<id>`). 복원만 하고 사바쨩이 지우지 않는다
const GAME_SNAPSHOT = 'game-snapshot';
const ACTIONS = ['start', 'stop', 'restart', 'save', 'backup', 'announce', 'update'];
// 게임 명령은 위 목록 대신 따로 띄우는 명령 목록에서 고른다
const COMMAND = 'command';
// 예고(warn_minutes)를 받는 작업 (데몬 problems()와 같다)
const WARNS = ['restart', 'update', 'stop'];
// 데몬 `scheduler::WEEKDAYS` (월요일부터)
export const WEEKDAYS = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'];
const MAX_EVERY_DAYS = 365;
const MODES = ['every', 'daily', 'days', 'weekdays', 'date'];

/** 이 화면을 띄울 IPC가 있는지 (로드맵 쪽 preload가 먼저 들어와야 한다) */
export const maintenanceAvailable = () => !!window.api?.instanceScheduleGet;

export function formatBytes(n) {
    if (!Number.isFinite(n) || n < 0) return '—';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    let i = 0;
    let v = n;
    while (v >= 1024 && i < units.length - 1) {
        v /= 1024;
        i += 1;
    }
    return `${i === 0 ? v : v.toFixed(v < 10 ? 1 : 0)} ${units[i]}`;
}

// 데몬 원문(영어) 대신 보일 문구가 있는 거절 코드
const CODE_TEXT = {
    restore_outside: 'maintenance.err_restore_outside',
    backup_busy: 'maintenance.err_backup_busy',
    // 켜진 서버가 쥐고 있는 파일 (`file`: 그 이름)
    backup_file_locked: 'maintenance.err_backup_file_locked',
    // 백업은 만들었지만 자동 저장을 되돌리지 못했다 ([backup] resume_command)
    autosave_not_resumed: 'maintenance.err_autosave_not_resumed',
    snapshot_not_found: 'maintenance.err_snapshot_not_found',
    schedule_changed: 'maintenance.err_schedule_changed',
    server_running: 'maintenance.restore_stop_first',
};

const errorText = (res, fallback, t) => {
    const details = Array.isArray(res?.details) ? res.details.filter(Boolean) : [];
    const known = CODE_TEXT[res?.error_code];
    return [known ? t(known, { file: res?.file || '' }) : res?.error || fallback, ...details]
        .filter(Boolean)
        .join('\n');
};

// ── 예약 작업 초안 ↔ 데몬 형식 ──
let draftSeq = 0;
const modeOf = (task) => {
    if (!task.at) return 'every';
    if (task.every_days != null) return 'days';
    if (task.weekdays?.length) return 'weekdays';
    if (task.date) return 'date';
    return 'daily';
};
export function toDraft(task) {
    draftSeq += 1;
    return {
        key: `t${draftSeq}`,
        action: task.action || 'restart',
        mode: modeOf(task),
        every: task.every_minutes != null ? String(task.every_minutes) : '60',
        at: task.at || '04:00',
        everyDays: task.every_days != null ? String(task.every_days) : '2',
        startDate: task.start_date || '',
        weekdays: task.weekdays || [],
        date: task.date || '',
        warn: (task.warn_minutes || []).join(', '),
        message: task.message || '',
        command: task.command || '',
        args: Object.fromEntries(Object.entries(task.args || {}).map(([k, v]) => [k, String(v)])),
        // 저장된 인자 값 (고치지 않은 값은 원래 형식 그대로 보낸다: 다른 곳에서 글자로 저장한 숫자 등)
        savedArgs: task.args || {},
        enabled: task.enabled !== false,
    };
}

const parseWarn = (text) =>
    text
        .split(/[\s,]+/)
        .map((w) => Number(w))
        .filter((n) => Number.isInteger(n) && n > 0);

/** `inputs`: 고른 게임 명령의 입력 정의 (숫자 입력은 숫자로 보낸다) */
export function fromDraft(d, { announce, inputs = [] }) {
    const task = { action: d.action, enabled: d.enabled };
    if (d.mode === 'every') task.every_minutes = Number(d.every);
    else task.at = d.at;
    if (d.mode === 'days') {
        task.every_days = Number(d.everyDays);
        if (d.startDate) task.start_date = d.startDate;
    }
    if (d.mode === 'weekdays') task.weekdays = WEEKDAYS.filter((w) => d.weekdays.includes(w));
    if (d.mode === 'date') task.date = d.date;
    if (WARNS.includes(d.action) && announce) {
        const warn = parseWarn(d.warn);
        if (warn.length) task.warn_minutes = warn;
    }
    if (d.action === 'announce') task.message = d.message.trim();
    if (d.action === COMMAND) {
        task.command = d.command;
        const args = {};
        for (const [name, raw] of Object.entries(d.args)) {
            const value = String(raw).trim();
            if (!value) continue;
            if (name in d.savedArgs && String(d.savedArgs[name]) === value) {
                args[name] = d.savedArgs[name];
                continue;
            }
            const type = inputs.find((i) => i.name === name)?.type;
            args[name] = type === 'number' && Number.isFinite(Number(value)) ? Number(value) : value;
        }
        if (Object.keys(args).length) task.args = args;
    }
    return task;
}

/** 저장 전에 화면에서 바로 알 수 있는 문제 (데몬 problems()와 같은 규칙). `command`: 고른 게임 명령의 정의 */
export function draftProblems(d, t, command) {
    const out = [];
    if (d.mode === 'every') {
        const n = Number(d.every);
        if (!Number.isInteger(n) || n < 1) out.push(t('maintenance.err_every'));
    } else if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(d.at)) {
        out.push(t('maintenance.err_at'));
    }
    if (d.mode === 'days') {
        const n = Number(d.everyDays);
        if (!Number.isInteger(n) || n < 1 || n > MAX_EVERY_DAYS) {
            out.push(t('maintenance.err_every_days', { max: MAX_EVERY_DAYS }));
        }
    }
    if (d.mode === 'weekdays' && !d.weekdays.length) out.push(t('maintenance.err_weekdays'));
    if (d.mode === 'date' && !/^\d{4}-\d{2}-\d{2}$/.test(d.date)) out.push(t('maintenance.err_date'));
    if (d.action === 'announce' && !d.message.trim()) out.push(t('maintenance.err_message'));
    if (d.action === COMMAND) {
        if (!d.command) out.push(t('maintenance.err_command'));
        for (const input of command?.inputs || []) {
            if (input.required && !String(d.args[input.name] ?? '').trim()) {
                out.push(t('maintenance.err_command_input', { name: input.label || input.name }));
            }
        }
    }
    return out;
}

/** 요일 이름 (보는 사람의 언어, 2024-01-01이 월요일) */
const weekdayLabel = (lang, i) =>
    new Intl.DateTimeFormat(lang, { weekday: 'short', timeZone: 'UTC' }).format(new Date(Date.UTC(2024, 0, 1 + i)));

/** UTC 차이(분) → "UTC+9", "UTC-3:30" */
const offsetText = (minutes) => {
    const sign = minutes < 0 ? '-' : '+';
    const abs = Math.abs(minutes);
    const h = Math.floor(abs / 60);
    const m = abs % 60;
    return `UTC${sign}${h}${m ? `:${String(m).padStart(2, '0')}` : ''}`;
};

/**
 * 서버별 백업·예약 작업 탭 (데몬 A2·A3).
 * 백업 목록은 모듈이 [backup]을 선언했을 때만, 공지·예고는 공지 명령이 있을 때만 보인다.
 */
export function MaintenanceTab({ server, moduleCommands = [] }) {
    const { t } = useTranslation('gui');
    const [info, setInfo] = useState(null);
    const [loadError, setLoadError] = useState(null);

    const load = useCallback(async () => {
        setLoadError(null);
        try {
            const res = await window.api.instanceScheduleGet(server.id);
            if (res?.success === false) throw new Error(errorText(res, t('maintenance.load_failed'), t));
            setInfo({
                schedule: Array.isArray(res?.schedule) ? res.schedule : [],
                backupKeep: res?.backup_keep ?? 10,
                backupSupported: !!res?.backup_supported,
                announceSupported: !!res?.announce_supported,
                saveSupported: !!res?.save_supported,
                // 작업마다 다음 실행 시각 (유닉스 초). 보는 사람의 시간대로 보인다
                nextRuns: Array.isArray(res?.next_runs) ? res.next_runs : [],
                // 작업 시각은 서버 PC의 시계다
                utcOffset: typeof res?.utc_offset_minutes === 'number' ? res.utc_offset_minutes : null,
                // 저장할 때 다시 보낸다: 그사이 다른 곳에서 바뀌었으면 데몬이 덮어쓰지 않고 거절한다 (L51)
                revision: res?.revision,
            });
        } catch (err) {
            setLoadError(err.message || String(err));
        }
    }, [server.id, t]);

    useEffect(() => {
        setInfo(null);
        load();
    }, [load]);

    if (loadError) {
        return <SabaFail message={loadError} onRetry={load} retryLabel={t('maintenance.retry')} />;
    }
    if (!info) {
        return (
            <div className="empty-inline">
                <SabaSpinner />
                <p>{t('shell.loading')}</p>
            </div>
        );
    }
    return (
        <div className="mt-tab">
            {info.backupSupported && window.api?.instanceBackups && <BackupsCard server={server} />}
            <ScheduleCard server={server} info={info} onSaved={load} moduleCommands={moduleCommands} />
        </div>
    );
}

function BackupsCard({ server }) {
    const { t, i18n } = useTranslation('gui');
    const [backups, setBackups] = useState(null);
    // 모듈이 게임 자동 백업을 알려 주는가 (`[backup] snapshots`)
    const [snapshots, setSnapshots] = useState(false);
    const [busy, setBusy] = useState(null); // 'create' | `restore:${name}` | `delete:${name}`
    const [armed, setArmed] = useState(null); // 두 번 눌러 확정할 동작
    const alive = useRef(true);
    useEffect(
        () => () => {
            alive.current = false;
        },
        [],
    );

    const refresh = useCallback(async () => {
        try {
            const res = await window.api.instanceBackups(server.id);
            if (!alive.current) return;
            setBackups(Array.isArray(res?.backups) ? res.backups : []);
            setSnapshots(res?.snapshots_supported === true);
        } catch {
            if (alive.current) setBackups([]);
        }
    }, [server.id]);

    useEffect(() => {
        refresh();
    }, [refresh]);

    const run = async (key, call, okMessage) => {
        setBusy(key);
        setArmed(null);
        try {
            const res = await call();
            if (res?.success === false) {
                safeShowToast(errorText(res, t('maintenance.action_failed'), t), 'error', 6000);
            } else if (okMessage) {
                // 일부만 된 결과(예: 되살리지 못한 링크)는 경고로 알린다
                const msg = okMessage(res);
                if (typeof msg === 'string') safeShowToast(msg, 'success', 4000);
                else safeShowToast(msg.text, msg.tone, 6000);
            }
        } catch (err) {
            safeShowToast(err.message || String(err), 'error', 6000);
        } finally {
            if (alive.current) {
                setBusy(null);
                refresh();
            }
        }
    };

    const stopped = server.status === 'stopped';
    const fmt = (s) =>
        new Date(s * 1000).toLocaleString(i18n.language, {
            year: 'numeric',
            month: 'numeric',
            day: 'numeric',
            hour: '2-digit',
            minute: '2-digit',
        });
    const list = [...(backups || [])].sort((a, b) => b.created - a.created);

    return (
        <section className="dc-card" aria-label={t('maintenance.backups_title')}>
            <div className="dc-card-head">
                <h2>{t('maintenance.backups_title')}</h2>
                {snapshots && (
                    <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        disabled={!!busy}
                        title={t('maintenance.backup_keep_snapshot_hint')}
                        onClick={() =>
                            run(
                                'keep',
                                () => window.api.instanceBackupCreate(server.id, { source: 'snapshot' }),
                                () => t('maintenance.backup_snapshot_kept'),
                            )
                        }
                    >
                        {busy === 'keep' && <SabaSpinner size="sm" />} {t('maintenance.backup_keep_snapshot')}
                    </button>
                )}
                <button
                    type="button"
                    className="btn btn-primary btn-sm"
                    disabled={!!busy}
                    onClick={() =>
                        run(
                            'create',
                            () => window.api.instanceBackupCreate(server.id),
                            (res) => (res?.backup ? t('maintenance.backup_created') : t('maintenance.backup_nothing')),
                        )
                    }
                >
                    {busy === 'create' ? <SabaSpinner size="sm" light /> : <Icon name="plus" size="sm" />}{' '}
                    {t('maintenance.backup_now')}
                </button>
            </div>
            <p className="dc-card-desc">{t('maintenance.backups_desc')}</p>
            {!stopped && (
                <p className="dc-field-error">
                    <Icon name="info" size="xs" /> {t('maintenance.restore_stop_first')}
                </p>
            )}
            {backups === null ? (
                <SabaSpinner />
            ) : list.length === 0 ? (
                <p className="muted">{t('maintenance.backups_empty')}</p>
            ) : (
                <ul className="dc-list">
                    {list.map((b) => {
                        const restoreKey = `restore:${b.name}`;
                        const deleteKey = `delete:${b.name}`;
                        // zip으로 열리지 않는 백업: 복원할 수 없고 지울 때까지 데몬이 쓰지 않는 내용을 정리하지 못한다
                        const unreadable = b.readable === false;
                        const game = b.kind === GAME_SNAPSHOT;
                        return (
                            <li key={b.name} className="dc-list-row">
                                <span className="dc-list-text">
                                    <span className="dc-list-name">{fmt(b.created)}</span>
                                    <span
                                        className={clsx('dc-list-sub', unreadable && 'warn')}
                                        title={unreadable ? t('maintenance.backup_unreadable_hint') : b.name}
                                    >
                                        {unreadable
                                            ? t('maintenance.backup_unreadable')
                                            : game
                                              ? t('maintenance.backup_game_snapshot')
                                              : typeof b.stored === 'number' && b.stored !== b.size
                                                ? t('maintenance.backup_size_stored', {
                                                      size: formatBytes(b.size),
                                                      stored: formatBytes(b.stored),
                                                  })
                                                : formatBytes(b.size)}{' '}
                                        · {b.name}
                                    </span>
                                </span>
                                <StatusChip
                                    tone={KIND_TONE[b.kind] || 'idle'}
                                    label={t(`maintenance.kind_${b.kind}`, { defaultValue: b.kind })}
                                />
                                <button
                                    type="button"
                                    className={clsx('btn btn-sm', armed === restoreKey ? 'btn-primary' : 'btn-ghost')}
                                    disabled={!stopped || !!busy || unreadable}
                                    title={
                                        unreadable
                                            ? t('maintenance.backup_unreadable_hint')
                                            : stopped
                                              ? t('maintenance.restore_hint')
                                              : t('maintenance.restore_stop_first')
                                    }
                                    onClick={() =>
                                        armed === restoreKey
                                            ? run(
                                                  restoreKey,
                                                  () => window.api.instanceBackupRestore(server.id, b.name),
                                                  (res) =>
                                                      res?.links_not_restored > 0
                                                          ? {
                                                                text: t('maintenance.restored_links_missed', {
                                                                    count: res.links_not_restored,
                                                                }),
                                                                tone: 'warning',
                                                            }
                                                          : t('maintenance.restored'),
                                              )
                                            : setArmed(restoreKey)
                                    }
                                >
                                    {busy === restoreKey && <SabaSpinner size="sm" />}
                                    {armed === restoreKey ? t('maintenance.restore_confirm') : t('maintenance.restore')}
                                </button>
                                {!game && (
                                    <button
                                        type="button"
                                        className={clsx('btn btn-sm', armed === deleteKey ? 'btn-danger' : 'btn-ghost')}
                                        disabled={!!busy}
                                        onClick={() =>
                                            armed === deleteKey
                                                ? run(deleteKey, () =>
                                                      window.api.instanceBackupDelete(server.id, b.name),
                                                  )
                                                : setArmed(deleteKey)
                                        }
                                    >
                                        {armed === deleteKey
                                            ? t('maintenance.delete_confirm')
                                            : t('maintenance.delete')}
                                    </button>
                                )}
                            </li>
                        );
                    })}
                </ul>
            )}
        </section>
    );
}

const findCommand = (commands, name) => commands.find((c) => c.name === name);
// 키 순서와 상관없이 같은 작업인지 견줄 문자열
const sameKey = (v) =>
    v && typeof v === 'object' && !Array.isArray(v)
        ? `{${Object.keys(v)
              .sort()
              .map((k) => `${JSON.stringify(k)}:${sameKey(v[k])}`)
              .join(',')}}`
        : JSON.stringify(v);

function ScheduleCard({ server, info, onSaved, moduleCommands }) {
    const { t, i18n } = useTranslation('gui');
    const base = useRef(null);
    const [drafts, setDrafts] = useState([]);
    const [keep, setKeep] = useState(String(info.backupKeep));
    const [saving, setSaving] = useState(false);
    const [serverErrors, setServerErrors] = useState([]);
    // 다른 곳에서 바뀌어 거절됨: 고친 내용은 두고 다시 읽기를 보인다
    const [stale, setStale] = useState(false);
    // 게임 명령 목록을 띄운 작업
    const [picking, setPicking] = useState(null);
    // 새로 넣은 위험 명령: 한 번 더 눌러 저장한다
    const [armed, setArmed] = useState(false);
    const commands = useInstanceCommands(server, moduleCommands) || [];

    useEffect(() => {
        const next = info.schedule.map(toDraft);
        setDrafts(next);
        setKeep(String(info.backupKeep));
        setServerErrors([]);
        setStale(false);
        setArmed(false);
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
    const toTask = (d) =>
        fromDraft(d, { announce: info.announceSupported, inputs: findCommand(commands, d.command)?.inputs });
    const saved = info.schedule.map(sameKey);

    // 저장된 그대로인 켜진 작업만 다음 실행 시각을 보인다 (고친 작업은 저장해야 정해진다)
    const nextRunOf = (d) => {
        const i = saved.indexOf(sameKey(toTask(d)));
        const at = i >= 0 ? info.nextRuns[i] : null;
        return typeof at === 'number' ? at : null;
    };
    const fmtRun = (secs) =>
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

    const patch = (key, change) => {
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
            const body = { schedule: drafts.map(toTask) };
            if (info.backupSupported) body.backup_keep = keepNum;
            if (info.revision) body.revision = info.revision;
            const res = await window.api.instanceScheduleSave(server.id, body);
            if (res?.success === false) {
                setStale(res.error_code === 'schedule_changed');
                setServerErrors(
                    Array.isArray(res.details) && res.details.length ? res.details : [errorText(res, 'error', t)],
                );
                return;
            }
            safeShowToast(t('maintenance.schedule_saved'), 'success', 3000);
            onSaved();
        } catch (err) {
            setServerErrors([err.message || String(err)]);
        } finally {
            setSaving(false);
        }
    };

    const pickingDraft = drafts.find((d) => d.key === picking);

    return (
        <section className="dc-card" aria-label={t('maintenance.schedule_title')}>
            <div className="dc-card-head">
                <h2>{t('maintenance.schedule_title')}</h2>
                <button
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
                    <Icon name="info" size="xs" /> {t('maintenance.server_clock', { zone: offsetText(info.utcOffset) })}
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
                        return (
                            <li key={d.key} className={clsx('mt-task', { off: !d.enabled })}>
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
                                                    <input
                                                        className="input mt-num"
                                                        type="number"
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
                                                    <input
                                                        className="input mt-num"
                                                        type="number"
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
                                                <input
                                                    className="input mt-date"
                                                    type="date"
                                                    value={d.date}
                                                    aria-label={t('maintenance.on_date')}
                                                    onChange={(e) => patch(d.key, { date: e.target.value })}
                                                />
                                            )}
                                            {d.mode !== 'every' && (
                                                <input
                                                    className="input mt-time"
                                                    type="time"
                                                    value={d.at}
                                                    aria-label={t('maintenance.at_time')}
                                                    onChange={(e) => patch(d.key, { at: e.target.value })}
                                                />
                                            )}
                                        </span>
                                    </label>
                                    {d.mode === 'days' && (
                                        <label className="mt-field">
                                            <span>{t('maintenance.start_date')}</span>
                                            <input
                                                className="input mt-date"
                                                type="date"
                                                value={d.startDate}
                                                title={t('maintenance.start_date_hint')}
                                                onChange={(e) => patch(d.key, { startDate: e.target.value })}
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
                                {next !== null && (
                                    <p className="muted mt-next">{t('maintenance.next_run', { time: fmtRun(next) })}</p>
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
                        <input
                            className="input mt-num"
                            type="number"
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

            <div className={clsx('dc-savebar', { dirty })}>
                <span className="dc-savebar-text">
                    {armed ? t('maintenance.danger_confirm') : dirty ? t('discord.unsaved') : ''}
                </span>
                {dirty && (
                    <button
                        type="button"
                        className="btn btn-ghost"
                        disabled={saving}
                        onClick={() => {
                            setDrafts(info.schedule.map(toDraft));
                            setKeep(String(info.backupKeep));
                            setServerErrors([]);
                            setArmed(false);
                        }}
                    >
                        {t('discord.revert')}
                    </button>
                )}
                <button
                    type="button"
                    className={clsx('btn', armed ? 'btn-danger' : 'btn-primary')}
                    disabled={!dirty || saving || blocked}
                    onClick={save}
                >
                    <Icon name="save" size="sm" /> {armed ? t('maintenance.danger_save') : t('discord.save')}
                </button>
            </div>
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

/** 게임 명령 입력 하나 (module.toml `[[commands.fields.inputs]]`) */
function CommandInput({ input, value, onChange }) {
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
                        ...input.options.map((o) => ({ value: String(o), label: String(o) })),
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
function CommandPicker({ commands, current, onPick, onClose }) {
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
        <div className="modal-overlay" onClick={onClose}>
            <div
                className="modal mt-picker"
                role="dialog"
                aria-label={t('maintenance.command_pick')}
                onClick={(e) => e.stopPropagation()}
                onKeyDown={(e) => {
                    if (e.key === 'Escape') onClose();
                }}
            >
                <h2 className="modal-title">{t('maintenance.command_pick')}</h2>
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
                    <ul className="mt-picker-list">
                        {list.map((c) => (
                            <li key={c.name}>
                                <button
                                    type="button"
                                    className={clsx('mt-picker-item', { selected: c.name === current })}
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
                <div className="modal-actions">
                    <button type="button" className="btn btn-ghost" onClick={onClose}>
                        {t('maintenance.close')}
                    </button>
                </div>
            </div>
        </div>
    );
}
