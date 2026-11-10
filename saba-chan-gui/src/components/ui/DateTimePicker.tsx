import clsx from 'clsx';
import { type KeyboardEvent, type ReactNode, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { Popover } from './Popover';
import './DateTimePicker.css';

const GAP = 4;

const pad = (n: number) => String(n).padStart(2, '0');
/** 로컬 날짜 → "YYYY-MM-DD" (input type=date와 같은 형식) */
const iso = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const parse = (s: string | null | undefined) => {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(s || '');
    return m ? new Date(+m[1], +m[2] - 1, +m[3]) : null;
};
const addDays = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);
const addMonths = (d: Date, n: number) => {
    // 31일에서 다음 달로 가면 그 달 마지막 날에 멈춘다
    const last = new Date(d.getFullYear(), d.getMonth() + n + 1, 0).getDate();
    return new Date(d.getFullYear(), d.getMonth() + n, Math.min(d.getDate(), last));
};

/** 주의 첫 요일 (0=일, 1=월). 언어 정보가 없으면 일요일 */
function firstWeekday(lang: string) {
    try {
        const loc = new Intl.Locale(lang);
        const weekLoc = loc as Intl.Locale & { getWeekInfo?: () => { firstDay?: number }; weekInfo?: { firstDay?: number } };
        const info = weekLoc.getWeekInfo?.() || weekLoc.weekInfo;
        if (info?.firstDay) return info.firstDay % 7;
    } catch {
        // 모르는 언어 태그
    }
    return 0;
}

interface TriggerProps {
    open: boolean;
    ariaLabel?: string;
    title?: string;
    disabled?: boolean;
    className?: string;
    icon: string;
    children?: ReactNode;
}

/** 값 칸처럼 보이는 여닫기 단추 (Popover의 trigger로 넘긴다) */
function trigger({ open, ariaLabel, title, disabled, className, icon, children }: TriggerProps) {
    return (
        <button
            type="button"
            className={clsx('input dtp-trigger', className, { open })}
            aria-label={ariaLabel}
            data-tooltip={title}
            disabled={disabled}
        >
            <span className="dtp-value">{children}</span>
            <Icon name={icon} size="sm" />
        </button>
    );
}

/**
 * 날짜 칸 (OS 기본 input type=date 대체). 값은 "YYYY-MM-DD" 또는 "".
 */
interface PickerFieldProps {
    value: string;
    onChange: (value: string) => void;
    ariaLabel?: string;
    title?: string;
    disabled?: boolean;
    className?: string;
}

interface DateFieldProps extends PickerFieldProps {
    /** "YYYY-MM-DD" */
    min?: string;
    max?: string;
    clearable?: boolean;
}

export function DateField({
    value,
    onChange,
    min,
    max,
    clearable,
    ariaLabel,
    title,
    disabled,
    className,
}: DateFieldProps) {
    const { t, i18n } = useTranslation('gui');
    const lang = i18n.language;
    const selected = parse(value);
    const [view, setView] = useState(() => selected || new Date());
    const [cursor, setCursor] = useState(() => selected || new Date());
    const gridRef = useRef<HTMLDivElement>(null);
    const [open, setOpen] = useState(false);
    const onOpenChange = (next: boolean) => {
        if (next) {
            const start = parse(value) || new Date();
            setView(start);
            setCursor(start);
        }
        setOpen(next);
    };
    const focusCursor = () => gridRef.current?.querySelector<HTMLElement>(`[data-date="${iso(cursor)}"]`)?.focus();

    const outOfRange = (d: Date) => (min && iso(d) < min) || (max && iso(d) > max);

    // 커서가 움직이면 그 날짜에 포커스 (열 때는 창이 그려질 때 onOpenAutoFocus에서)
    // biome-ignore lint/correctness/useExhaustiveDependencies: 커서가 바뀔 때만
    useEffect(() => {
        if (open) focusCursor();
    }, [cursor]);

    const pick = (d: Date) => {
        if (outOfRange(d)) return;
        if (iso(d) !== value) onChange(iso(d));
        setOpen(false);
    };

    const moveCursor = (d: Date) => {
        setCursor(d);
        if (d.getMonth() !== view.getMonth() || d.getFullYear() !== view.getFullYear()) setView(d);
    };

    const onGridKeyDown = (e: KeyboardEvent) => {
        const step = ({ ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 } as Record<string, number>)[e.key];
        if (step) {
            e.preventDefault();
            moveCursor(addDays(cursor, step));
        } else if (e.key === 'PageUp' || e.key === 'PageDown') {
            e.preventDefault();
            moveCursor(addMonths(cursor, e.key === 'PageUp' ? -1 : 1));
        }
    };

    const shiftView = (n: number) => {
        const next = addMonths(new Date(view.getFullYear(), view.getMonth(), 1), n);
        setView(next);
        setCursor(addMonths(cursor, n));
    };

    // 6주 42칸: 그 달 1일이 들어가는 주의 첫 요일부터
    const first = new Date(view.getFullYear(), view.getMonth(), 1);
    const weekStart = firstWeekday(lang);
    const gridStart = addDays(first, -((first.getDay() - weekStart + 7) % 7));
    const days = Array.from({ length: 42 }, (_, i) => addDays(gridStart, i));
    const weekdayFmt = new Intl.DateTimeFormat(lang, { weekday: 'narrow' });
    const longFmt = new Intl.DateTimeFormat(lang, { dateStyle: 'long' });
    const todayIso = iso(new Date());

    const label = selected
        ? new Intl.DateTimeFormat(lang, { year: 'numeric', month: '2-digit', day: '2-digit' }).format(selected)
        : null;

    return (
        <Popover
            open={open}
            onOpenChange={onOpenChange}
            trigger={trigger({
                open,
                ariaLabel,
                title,
                disabled,
                className,
                icon: 'calendar',
                children: label || <span className="dtp-placeholder">{t('picker.pick_date')}</span>,
            })}
            label={ariaLabel || t('picker.pick_date')}
            className="dtp-panel"
            align="start"
            sideOffset={GAP}
            onOpenAutoFocus={(e) => {
                e.preventDefault();
                focusCursor();
            }}
        >
            <div className="dtp-head">
                <span className="dtp-title">
                    {new Intl.DateTimeFormat(lang, { year: 'numeric', month: 'long' }).format(view)}
                </span>
                <button
                    type="button"
                    className="icon-btn"
                    aria-label={t('picker.prev_month')}
                    onClick={() => shiftView(-1)}
                >
                    <Icon name="chevronLeft" size="sm" />
                </button>
                <button
                    type="button"
                    className="icon-btn"
                    aria-label={t('picker.next_month')}
                    onClick={() => shiftView(1)}
                >
                    <Icon name="chevronRight" size="sm" />
                </button>
            </div>
            <div className="dtp-weekdays" aria-hidden="true">
                {days.slice(0, 7).map((d) => (
                    <span key={d.getDay()} className={clsx({ sun: d.getDay() === 0, sat: d.getDay() === 6 })}>
                        {weekdayFmt.format(d)}
                    </span>
                ))}
            </div>
            {/* biome-ignore lint/a11y/useSemanticElements: 날짜 버튼 묶음, 방향키는 여기서 받는다 */}
            <div className="dtp-days" role="group" ref={gridRef} onKeyDown={onGridKeyDown}>
                {days.map((d) => {
                    const key = iso(d);
                    return (
                        <button
                            key={key}
                            type="button"
                            data-date={key}
                            tabIndex={key === iso(cursor) ? 0 : -1}
                            aria-label={longFmt.format(d)}
                            aria-pressed={key === value}
                            aria-current={key === todayIso ? 'date' : undefined}
                            disabled={!!outOfRange(d)}
                            className={clsx('dtp-day', {
                                other: d.getMonth() !== view.getMonth(),
                                selected: key === value,
                                today: key === todayIso,
                                sun: d.getDay() === 0,
                                sat: d.getDay() === 6,
                            })}
                            onClick={() => pick(d)}
                        >
                            {d.getDate()}
                        </button>
                    );
                })}
            </div>
            <div className="dtp-foot">
                {clearable && (
                    <button
                        type="button"
                        className="btn btn-sm btn-ghost"
                        onClick={() => {
                            if (value) onChange('');
                            setOpen(false);
                        }}
                    >
                        {t('picker.clear')}
                    </button>
                )}
                <button
                    type="button"
                    className="btn btn-sm btn-ghost dtp-today"
                    disabled={!!outOfRange(new Date())}
                    onClick={() => pick(new Date())}
                >
                    {t('picker.today')}
                </button>
            </div>
        </Popover>
    );
}

const HOURS = Array.from({ length: 24 }, (_, i) => pad(i));
const MINUTES = Array.from({ length: 60 }, (_, i) => pad(i));

/**
 * 시간 칸 (OS 기본 input type=time 대체). 값은 24시간 "HH:MM" 또는 "".
 * 시를 고르면 그대로 열려 있고, 분을 고르면 닫힌다.
 */
export function TimeField({ value, onChange, ariaLabel, title, disabled, className }: PickerFieldProps) {
    const { t } = useTranslation('gui');
    const m = /^(\d{2}):(\d{2})/.exec(value || '');
    const hour = m?.[1];
    const minute = m?.[2];
    const panelRef = useRef<HTMLDivElement>(null);
    const [open, setOpen] = useState(false);

    // 열리면 고른 시·분을 보이게 하고 시 칸에 포커스
    const onOpened = (e: Event) => {
        e.preventDefault();
        const root = panelRef.current;
        for (const sel of [`[data-hour="${hour}"]`, `[data-minute="${minute}"]`]) {
            root?.querySelector(sel)?.scrollIntoView?.({ block: 'center' });
        }
        ((root?.querySelector(`[data-hour="${hour}"]`) || root?.querySelector('[data-hour]')) as HTMLElement | null)?.focus();
    };

    const set = (h: string, mm: string) => {
        const next = `${h}:${mm}`;
        if (next !== value) onChange(next);
    };

    // 방향키: 위·아래는 칸 안에서, 좌·우는 시↔분
    const onKeyDown = (e: KeyboardEvent) => {
        const btn = (e.target as Element).closest?.<HTMLElement>('[data-hour],[data-minute]');
        if (!btn) return;
        const col = btn.parentElement;
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
            e.preventDefault();
            const sib = (e.key === 'ArrowDown' ? btn.nextElementSibling : btn.previousElementSibling) as HTMLElement | null;
            sib?.focus();
            sib?.scrollIntoView?.({ block: 'nearest' });
        } else if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
            e.preventDefault();
            const other = panelRef.current?.querySelectorAll('.dtp-col')[e.key === 'ArrowLeft' ? 0 : 1];
            if (other && other !== col)
                ((other.querySelector('[aria-selected="true"]') || other.firstChild) as HTMLElement | null)?.focus();
        }
    };

    return (
        <Popover
            open={open}
            onOpenChange={setOpen}
            trigger={trigger({
                open,
                ariaLabel,
                title,
                disabled,
                className,
                icon: 'clock',
                children: m ? `${hour}:${minute}` : <span className="dtp-placeholder">{t('picker.pick_time')}</span>,
            })}
            label={ariaLabel || t('picker.pick_time')}
            className="dtp-panel dtp-time"
            align="start"
            sideOffset={GAP}
            onOpenAutoFocus={onOpened}
        >
            {/* biome-ignore lint/a11y/noStaticElementInteractions: 방향키를 두 칸에서 함께 받는다 */}
            <div className="dtp-cols" ref={panelRef} onKeyDown={onKeyDown}>
                <div className="dtp-col" role="listbox" aria-label={t('picker.hour')}>
                    {HOURS.map((h) => (
                        <button
                            key={h}
                            type="button"
                            role="option"
                            data-hour={h}
                            aria-selected={h === hour}
                            className={clsx('dtp-cell', { selected: h === hour })}
                            onClick={() => set(h, minute || '00')}
                        >
                            {h}
                        </button>
                    ))}
                </div>
                <div className="dtp-col" role="listbox" aria-label={t('picker.minute')}>
                    {MINUTES.map((mm) => (
                        <button
                            key={mm}
                            type="button"
                            role="option"
                            data-minute={mm}
                            aria-selected={mm === minute}
                            className={clsx('dtp-cell', { selected: mm === minute })}
                            onClick={() => {
                                set(hour || '00', mm);
                                setOpen(false);
                            }}
                        >
                            {mm}
                        </button>
                    ))}
                </div>
            </div>
        </Popover>
    );
}
