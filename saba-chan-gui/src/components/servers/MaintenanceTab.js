import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../../utils/helpers';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { Segmented, StatusChip } from '../ui/StatusChip';

const KIND_TONE = { manual: 'info', scheduled: 'idle', 'pre-update': 'warn', 'pre-restore': 'warn' };
const ACTIONS = ['backup', 'restart', 'announce', 'update'];

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

const errorText = (res, fallback) => {
    const details = Array.isArray(res?.details) ? res.details.filter(Boolean) : [];
    return [res?.error || fallback, ...details].filter(Boolean).join('\n');
};

// ── 예약 작업 초안 ↔ 데몬 형식 ──
let draftSeq = 0;
export function toDraft(task) {
    draftSeq += 1;
    return {
        key: `t${draftSeq}`,
        action: task.action || 'restart',
        mode: task.at ? 'at' : 'every',
        every: task.every_minutes != null ? String(task.every_minutes) : '60',
        at: task.at || '04:00',
        warn: (task.warn_minutes || []).join(', '),
        message: task.message || '',
        enabled: task.enabled !== false,
    };
}

const parseWarn = (text) =>
    text
        .split(/[\s,]+/)
        .map((w) => Number(w))
        .filter((n) => Number.isInteger(n) && n > 0);

export function fromDraft(d, { announce }) {
    const task = { action: d.action, enabled: d.enabled };
    if (d.mode === 'at') task.at = d.at;
    else task.every_minutes = Number(d.every);
    if ((d.action === 'restart' || d.action === 'update') && announce) {
        const warn = parseWarn(d.warn);
        if (warn.length) task.warn_minutes = warn;
    }
    if (d.action === 'announce') task.message = d.message.trim();
    return task;
}

/** 저장 전에 화면에서 바로 알 수 있는 문제 (데몬 problems()와 같은 규칙) */
export function draftProblems(d, t) {
    const out = [];
    if (d.mode === 'every') {
        const n = Number(d.every);
        if (!Number.isInteger(n) || n < 1) out.push(t('maintenance.err_every'));
    } else if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(d.at)) {
        out.push(t('maintenance.err_at'));
    }
    if (d.action === 'announce' && !d.message.trim()) out.push(t('maintenance.err_message'));
    return out;
}

/**
 * 서버별 백업·예약 작업 탭 (데몬 A2·A3).
 * 백업 목록은 모듈이 [backup]을 선언했을 때만, 공지·예고는 공지 명령이 있을 때만 보인다.
 */
export function MaintenanceTab({ server }) {
    const { t } = useTranslation('gui');
    const [info, setInfo] = useState(null);
    const [loadError, setLoadError] = useState(null);

    const load = useCallback(async () => {
        setLoadError(null);
        try {
            const res = await window.api.instanceScheduleGet(server.id);
            if (res?.success === false) throw new Error(errorText(res, t('maintenance.load_failed')));
            setInfo({
                schedule: Array.isArray(res?.schedule) ? res.schedule : [],
                backupKeep: res?.backup_keep ?? 10,
                backupSupported: !!res?.backup_supported,
                announceSupported: !!res?.announce_supported,
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
        return (
            <div className="notice tone-warn" role="alert">
                <Icon name="alertTriangle" size="sm" />
                <span className="mt-pre">{loadError}</span>
                <button type="button" className="btn btn-ghost btn-sm" onClick={load}>
                    {t('maintenance.retry')}
                </button>
            </div>
        );
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
            <ScheduleCard server={server} info={info} onSaved={load} />
        </div>
    );
}

function BackupsCard({ server }) {
    const { t, i18n } = useTranslation('gui');
    const [backups, setBackups] = useState(null);
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
                safeShowToast(errorText(res, t('maintenance.action_failed')), 'error', 6000);
            } else if (okMessage) {
                safeShowToast(okMessage(res), 'success', 4000);
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
                        return (
                            <li key={b.name} className="dc-list-row">
                                <span className="dc-list-text">
                                    <span className="dc-list-name">{fmt(b.created)}</span>
                                    <span className="dc-list-sub" title={b.name}>
                                        {formatBytes(b.size)} · {b.name}
                                    </span>
                                </span>
                                <StatusChip
                                    tone={KIND_TONE[b.kind] || 'idle'}
                                    label={t(`maintenance.kind_${b.kind}`, { defaultValue: b.kind })}
                                />
                                <button
                                    type="button"
                                    className={clsx('btn btn-sm', armed === restoreKey ? 'btn-primary' : 'btn-ghost')}
                                    disabled={!stopped || !!busy}
                                    title={
                                        stopped ? t('maintenance.restore_hint') : t('maintenance.restore_stop_first')
                                    }
                                    onClick={() =>
                                        armed === restoreKey
                                            ? run(
                                                  restoreKey,
                                                  () => window.api.instanceBackupRestore(server.id, b.name),
                                                  () => t('maintenance.restored'),
                                              )
                                            : setArmed(restoreKey)
                                    }
                                >
                                    {busy === restoreKey && <SabaSpinner size="sm" />}
                                    {armed === restoreKey ? t('maintenance.restore_confirm') : t('maintenance.restore')}
                                </button>
                                <button
                                    type="button"
                                    className={clsx('btn btn-sm', armed === deleteKey ? 'btn-danger' : 'btn-ghost')}
                                    disabled={!!busy}
                                    onClick={() =>
                                        armed === deleteKey
                                            ? run(deleteKey, () => window.api.instanceBackupDelete(server.id, b.name))
                                            : setArmed(deleteKey)
                                    }
                                >
                                    {armed === deleteKey ? t('maintenance.delete_confirm') : t('maintenance.delete')}
                                </button>
                            </li>
                        );
                    })}
                </ul>
            )}
        </section>
    );
}

function ScheduleCard({ server, info, onSaved }) {
    const { t } = useTranslation('gui');
    const base = useRef(null);
    const [drafts, setDrafts] = useState([]);
    const [keep, setKeep] = useState(String(info.backupKeep));
    const [saving, setSaving] = useState(false);
    const [serverErrors, setServerErrors] = useState([]);

    useEffect(() => {
        const next = info.schedule.map(toDraft);
        setDrafts(next);
        setKeep(String(info.backupKeep));
        base.current = JSON.stringify({ d: next.map(({ key, ...r }) => r), k: String(info.backupKeep) });
    }, [info]);

    const snapshot = JSON.stringify({ d: drafts.map(({ key, ...r }) => r), k: keep });
    const dirty = base.current !== null && snapshot !== base.current;

    const actions = ACTIONS.filter(
        (a) => (a !== 'backup' || info.backupSupported) && (a !== 'announce' || info.announceSupported),
    );
    const keepNum = Number(keep);
    const keepBad = info.backupSupported && (!Number.isInteger(keepNum) || keepNum < 1);
    const problems = drafts.map((d) => draftProblems(d, t));
    const blocked = keepBad || problems.some((p) => p.length > 0);

    const patch = (key, change) => setDrafts((ds) => ds.map((d) => (d.key === key ? { ...d, ...change } : d)));

    const save = async () => {
        setSaving(true);
        setServerErrors([]);
        try {
            const body = { schedule: drafts.map((d) => fromDraft(d, { announce: info.announceSupported })) };
            if (info.backupSupported) body.backup_keep = keepNum;
            const res = await window.api.instanceScheduleSave(server.id, body);
            if (res?.success === false) {
                setServerErrors(
                    Array.isArray(res.details) && res.details.length ? res.details : [res.error || 'error'],
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

            {drafts.length === 0 ? (
                <p className="muted">{t('maintenance.schedule_empty')}</p>
            ) : (
                <ul className="mt-tasks">
                    {drafts.map((d, i) => {
                        const items = (actions.includes(d.action) ? actions : [...actions, d.action]).map((a) => ({
                            id: a,
                            label: t(`maintenance.action_${a}`, { defaultValue: a }),
                        }));
                        const canWarn = (d.action === 'restart' || d.action === 'update') && info.announceSupported;
                        return (
                            <li key={d.key} className={clsx('mt-task', { off: !d.enabled })}>
                                <div className="mt-task-head">
                                    <Segmented
                                        items={items}
                                        value={d.action}
                                        onChange={(a) => patch(d.key, { action: a })}
                                        ariaLabel={t('maintenance.task_aria', { n: i + 1 })}
                                    />
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
                                                options={[
                                                    { value: 'every', label: t('maintenance.mode_every') },
                                                    { value: 'at', label: t('maintenance.mode_at') },
                                                ]}
                                            />
                                            {d.mode === 'every' ? (
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
                                            ) : (
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
                                </div>
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
                </div>
            )}

            <div className={clsx('dc-savebar', { dirty })}>
                <span className="dc-savebar-text">{dirty ? t('discord.unsaved') : ''}</span>
                {dirty && (
                    <button
                        type="button"
                        className="btn btn-ghost"
                        disabled={saving}
                        onClick={() => {
                            setDrafts(info.schedule.map(toDraft));
                            setKeep(String(info.backupKeep));
                            setServerErrors([]);
                        }}
                    >
                        {t('discord.revert')}
                    </button>
                )}
                <button type="button" className="btn btn-primary" disabled={!dirty || saving || blocked} onClick={save}>
                    <Icon name="save" size="sm" /> {t('discord.save')}
                </button>
            </div>
        </section>
    );
}
