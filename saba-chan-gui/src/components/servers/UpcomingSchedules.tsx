import clsx from 'clsx';
import type { TFunction } from 'i18next';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ScheduledTask } from '../../utils/schedule';
import { WEEKDAYS, weekdayLabel } from '../../utils/schedule';
import { failed } from '../../utils/apiResult';
import { openScreen } from '../../utils/openScreen';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaSpinner } from '../ui/SabaUI';
import { StatusChip } from '../ui/StatusChip';

/** 이 카드를 띄울 IPC가 있는지 (구버전 브리지에는 없다) */
export const upcomingAvailable = () => typeof window.api?.schedulesUpcoming === 'function';

/** 데몬 GET /api/schedules/upcoming 의 한 줄 */
interface UpcomingRow {
    instance_id: string;
    instance: string;
    index: number;
    next: number;
    clash: boolean;
    task: ScheduledTask;
}

interface Upcoming {
    rows: UpcomingRow[];
    clashes: { at: number; actions: string[] }[];
}

const RANGES = [24, 168, 720];

const actionLabel = (task: ScheduledTask, t: TFunction) =>
    task.action === 'command' && task.command
        ? task.command
        : t(`maintenance.action_${task.action}`, { defaultValue: task.action });

/** 작업 → "매일", "요일마다 (화·금)", "30분마다" */
function repeatText(task: ScheduledTask, t: TFunction, lang: string) {
    if (task.every_minutes) return t('audit.upcoming_every_minutes', { n: task.every_minutes });
    if (task.date) return t('audit.upcoming_once');
    if (task.every_days) return t('audit.upcoming_every_days', { n: task.every_days });
    if (task.weekdays?.length) {
        const days = WEEKDAYS.filter((w) => task.weekdays?.includes(w)).map((w) => weekdayLabel(lang, WEEKDAYS.indexOf(w)));
        return `${t('maintenance.mode_weekdays')} (${days.join('·')})`;
    }
    return t('maintenance.mode_daily');
}

/** 유닉스 초 → "오늘 23:50", "내일 04:00", 그 뒤는 날짜와 시각 (보는 곳의 시간대) */
function whenText(secs: number, lang: string) {
    const at = new Date(secs * 1000);
    const time = at.toLocaleTimeString(lang, { hour: '2-digit', minute: '2-digit' });
    const midnight = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
    const days = Math.round((midnight(at) - midnight(new Date())) / 86_400_000);
    const day =
        days === 0 || days === 1
            ? new Intl.RelativeTimeFormat(lang, { numeric: 'auto' }).format(days, 'day')
            : at.toLocaleDateString(lang, { month: 'numeric', day: 'numeric', weekday: 'short' });
    return `${day} ${time}`;
}

/**
 * 다가오는 예약 (작업 기록 화면 맨 위): 이 PC 모든 서버의 다음 예약을 시각 순서로 보인다.
 * 겹침 판정과 순서는 데몬이 정한다. 줄을 누르면 그 서버의 백업·예약 탭으로 간다.
 */
export function UpcomingSchedules() {
    const { t, i18n } = useTranslation('gui');
    const [hours, setHours] = useState(RANGES[0]);
    const [data, setData] = useState<Upcoming | null>(null);
    const [error, setError] = useState<string | null>(null);
    const seq = useRef(0); // 늦게 온 이전 응답이 새 응답을 덮지 않게

    const load = useCallback(async () => {
        const mine = ++seq.current;
        try {
            const res = await window.api.schedulesUpcoming(hours);
            if (mine !== seq.current) return;
            if (failed(res) || !Array.isArray(res?.upcoming)) throw new Error(res?.error || t('audit.upcoming_load_failed'));
            setError(null);
            setData({ rows: res.upcoming, clashes: Array.isArray(res.clashes) ? res.clashes : [] });
        } catch (err) {
            if (mine === seq.current) setError((err as Error).message || String(err));
        }
    }, [hours, t]);

    useEffect(() => {
        load();
    }, [load]);

    return (
        <section className="dc-card upcoming-card" aria-label={t('audit.upcoming_title')}>
            <div className="dc-card-head">
                <h2>{t('audit.upcoming_title')}</h2>
                <CustomDropdown
                    className="upcoming-range"
                    value={hours}
                    ariaLabel={t('audit.upcoming_range')}
                    onChange={(v: number) => setHours(v)}
                    options={RANGES.map((h) => ({ value: h, label: t(`audit.upcoming_range_${h}`) }))}
                />
            </div>
            <p className="dc-card-desc">{t('audit.upcoming_desc')}</p>
            {error && (
                <div className="notice tone-warn" role="alert">
                    <Icon name="alertTriangle" size="sm" />
                    <span>{error}</span>
                </div>
            )}
            {data?.clashes.map((c) => (
                <div key={c.at} className="notice tone-warn" role="alert">
                    <Icon name="alertTriangle" size="sm" />
                    <span>
                        {t('audit.upcoming_clash', {
                            time: whenText(c.at, i18n.language),
                            actions: c.actions
                                .map((a) => t(`maintenance.action_${a}`, { defaultValue: a }))
                                .join(', '),
                        })}
                    </span>
                </div>
            ))}
            {data === null ? (
                !error && <SabaSpinner />
            ) : data.rows.length === 0 ? (
                <p className="muted">{t('audit.upcoming_empty')}</p>
            ) : (
                <ul className="dc-list upcoming-list">
                    {data.rows.map((r) => (
                        <li key={`${r.instance_id}-${r.index}`}>
                            <button
                                type="button"
                                className={clsx('dc-list-row is-openable', { clash: r.clash })}
                                onClick={() => openScreen(`server/${r.instance_id}/maintenance`)}
                            >
                                <span className="dc-list-text">
                                    <span className="dc-list-name">
                                        {whenText(r.next, i18n.language)} · {r.instance}
                                    </span>
                                    <span className="dc-list-sub">
                                        {r.task.action === 'announce' && r.task.message
                                            ? `${actionLabel(r.task, t)} "${r.task.message}"`
                                            : `${actionLabel(r.task, t)} · ${repeatText(r.task, t, i18n.language)}`}
                                    </span>
                                </span>
                                <StatusChip tone="idle" label={actionLabel(r.task, t)} />
                            </button>
                        </li>
                    ))}
                </ul>
            )}
        </section>
    );
}
