import clsx from 'clsx';
import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import ReactDOM from 'react-dom';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import './DateTimePicker.css';

const GAP = 4;
const CLOSE_MS = 120;

const pad = (n) => String(n).padStart(2, '0');
/** 로컬 날짜 → "YYYY-MM-DD" (input type=date와 같은 형식) */
const iso = (d) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const parse = (s) => {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(s || '');
    return m ? new Date(+m[1], +m[2] - 1, +m[3]) : null;
};
const addDays = (d, n) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);
const addMonths = (d, n) => {
    // 31일에서 다음 달로 가면 그 달 마지막 날에 멈춘다
    const last = new Date(d.getFullYear(), d.getMonth() + n + 1, 0).getDate();
    return new Date(d.getFullYear(), d.getMonth() + n, Math.min(d.getDate(), last));
};

/** 주의 첫 요일 (0=일, 1=월). 언어 정보가 없으면 일요일 */
function firstWeekday(lang) {
    try {
        const loc = new Intl.Locale(lang);
        const info = loc.getWeekInfo?.() || loc.weekInfo;
        if (info?.firstDay) return info.firstDay % 7;
    } catch {
        // 모르는 언어 태그
    }
    return 0;
}

/**
 * 트리거 아래(자리가 없으면 위)에 띄우는 팝업. CustomDropdown과 같은 열고 닫기·바깥 클릭 규칙.
 * 열 때 onOpen, 닫힌 뒤 트리거로 포커스를 돌린다.
 */
function usePopover(onOpen) {
    const [isOpen, setIsOpen] = useState(false);
    const [isClosing, setIsClosing] = useState(false);
    const [style, setStyle] = useState({});
    const [placement, setPlacement] = useState('bottom');
    const triggerRef = useRef(null);
    const panelRef = useRef(null);
    const closeTimer = useRef(null);

    const place = useCallback(() => {
        if (!triggerRef.current) return;
        const rect = triggerRef.current.getBoundingClientRect();
        const height = panelRef.current?.offsetHeight || 320;
        const below = window.innerHeight - rect.bottom - GAP;
        const up = below < height && rect.top - GAP > below;
        const alignRight = rect.left + rect.width / 2 > window.innerWidth / 2;
        setPlacement(up ? 'top' : 'bottom');
        setStyle({
            position: 'fixed',
            zIndex: 10000,
            ...(alignRight ? { right: window.innerWidth - rect.right } : { left: rect.left }),
            ...(up ? { bottom: window.innerHeight - rect.top + GAP } : { top: rect.bottom + GAP }),
        });
    }, []);

    const open = () => {
        clearTimeout(closeTimer.current);
        setIsClosing(false);
        onOpen?.();
        setIsOpen(true);
    };
    const close = useCallback((refocus = true) => {
        setIsClosing(true);
        clearTimeout(closeTimer.current);
        closeTimer.current = setTimeout(() => {
            setIsOpen(false);
            setIsClosing(false);
        }, CLOSE_MS);
        if (refocus) triggerRef.current?.focus();
    }, []);

    useEffect(() => () => clearTimeout(closeTimer.current), []);

    useLayoutEffect(() => {
        if (!isOpen) return;
        place();
        window.addEventListener('scroll', place, true);
        window.addEventListener('resize', place);
        return () => {
            window.removeEventListener('scroll', place, true);
            window.removeEventListener('resize', place);
        };
    }, [isOpen, place]);

    useEffect(() => {
        if (!isOpen || isClosing) return;
        const outside = (e) => {
            if (triggerRef.current?.contains(e.target) || panelRef.current?.contains(e.target)) return;
            close(false);
        };
        document.addEventListener('mousedown', outside);
        return () => document.removeEventListener('mousedown', outside);
    }, [isOpen, isClosing, close]);

    const shown = isOpen && !isClosing;
    const onPanelKeyDown = (e) => {
        if (e.key === 'Escape') {
            e.preventDefault();
            e.stopPropagation();
            close();
        } else if (e.key === 'Tab') {
            close(false);
        }
    };
    const panelProps = {
        ref: panelRef,
        style,
        className: clsx('dtp-panel', `placement-${placement}`, { closing: isClosing }),
        onKeyDown: onPanelKeyDown,
    };
    return { isOpen, shown, open, close, triggerRef, panelProps };
}

function Trigger({ id, controls, popover, ariaLabel, title, disabled, className, icon, children }) {
    return (
        <button
            id={id}
            ref={popover.triggerRef}
            type="button"
            className={clsx('input dtp-trigger', className, { open: popover.shown })}
            onClick={() => (popover.shown ? popover.close() : popover.open())}
            aria-label={ariaLabel}
            title={title}
            aria-haspopup="dialog"
            aria-expanded={popover.shown}
            aria-controls={controls}
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
export function DateField({ value, onChange, min, max, clearable, ariaLabel, title, disabled, className }) {
    const { t, i18n } = useTranslation('gui');
    const lang = i18n.language;
    const selected = parse(value);
    const [view, setView] = useState(() => selected || new Date());
    const [cursor, setCursor] = useState(() => selected || new Date());
    const gridRef = useRef(null);
    const autoId = useId();
    const panelId = `date-${autoId}`;

    const popover = usePopover(() => {
        const start = parse(value) || new Date();
        setView(start);
        setCursor(start);
    });

    const outOfRange = (d) => (min && iso(d) < min) || (max && iso(d) > max);

    // 열리거나 커서가 움직이면 그 날짜에 포커스
    useEffect(() => {
        if (!popover.shown) return;
        gridRef.current?.querySelector(`[data-date="${iso(cursor)}"]`)?.focus();
    }, [popover.shown, cursor]);

    const pick = (d) => {
        if (outOfRange(d)) return;
        if (iso(d) !== value) onChange(iso(d));
        popover.close();
    };

    const moveCursor = (d) => {
        setCursor(d);
        if (d.getMonth() !== view.getMonth() || d.getFullYear() !== view.getFullYear()) setView(d);
    };

    const onGridKeyDown = (e) => {
        const step = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 }[e.key];
        if (step) {
            e.preventDefault();
            moveCursor(addDays(cursor, step));
        } else if (e.key === 'PageUp' || e.key === 'PageDown') {
            e.preventDefault();
            moveCursor(addMonths(cursor, e.key === 'PageUp' ? -1 : 1));
        }
    };

    const shiftView = (n) => {
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
        <>
            <Trigger
                controls={panelId}
                popover={popover}
                ariaLabel={ariaLabel}
                title={title}
                disabled={disabled}
                className={className}
                icon="calendar"
            >
                {label || <span className="dtp-placeholder">{t('picker.pick_date')}</span>}
            </Trigger>
            {popover.isOpen &&
                ReactDOM.createPortal(
                    <div {...popover.panelProps} id={panelId} role="dialog" aria-label={ariaLabel}>
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
                                <span
                                    key={d.getDay()}
                                    className={clsx({ sun: d.getDay() === 0, sat: d.getDay() === 6 })}
                                >
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
                                        popover.close();
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
                    </div>,
                    document.body,
                )}
        </>
    );
}

const HOURS = Array.from({ length: 24 }, (_, i) => pad(i));
const MINUTES = Array.from({ length: 60 }, (_, i) => pad(i));

/**
 * 시간 칸 (OS 기본 input type=time 대체). 값은 24시간 "HH:MM" 또는 "".
 * 시를 고르면 그대로 열려 있고, 분을 고르면 닫힌다.
 */
export function TimeField({ value, onChange, ariaLabel, title, disabled, className }) {
    const { t } = useTranslation('gui');
    const m = /^(\d{2}):(\d{2})/.exec(value || '');
    const hour = m?.[1];
    const minute = m?.[2];
    const panelRef = useRef(null);
    const autoId = useId();
    const panelId = `time-${autoId}`;
    const popover = usePopover();

    // 열리면 고른 시·분을 보이게 하고 시 칸에 포커스
    // biome-ignore lint/correctness/useExhaustiveDependencies: 열릴 때 한 번만
    useEffect(() => {
        if (!popover.shown) return;
        const root = document.getElementById(panelId);
        for (const sel of [`[data-hour="${hour}"]`, `[data-minute="${minute}"]`]) {
            root?.querySelector(sel)?.scrollIntoView?.({ block: 'center' });
        }
        (root?.querySelector(`[data-hour="${hour}"]`) || root?.querySelector('[data-hour]'))?.focus();
    }, [popover.shown]);

    const set = (h, mm) => {
        const next = `${h}:${mm}`;
        if (next !== value) onChange(next);
    };

    // 방향키: 위·아래는 칸 안에서, 좌·우는 시↔분
    const onKeyDown = (e) => {
        const btn = e.target.closest?.('[data-hour],[data-minute]');
        if (!btn) return;
        const col = btn.parentElement;
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
            e.preventDefault();
            const sib = e.key === 'ArrowDown' ? btn.nextElementSibling : btn.previousElementSibling;
            sib?.focus();
            sib?.scrollIntoView?.({ block: 'nearest' });
        } else if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
            e.preventDefault();
            const other = panelRef.current?.querySelectorAll('.dtp-col')[e.key === 'ArrowLeft' ? 0 : 1];
            if (other && other !== col) (other.querySelector('[aria-selected="true"]') || other.firstChild)?.focus();
        }
    };

    return (
        <>
            <Trigger
                controls={panelId}
                popover={popover}
                ariaLabel={ariaLabel}
                title={title}
                disabled={disabled}
                className={className}
                icon="clock"
            >
                {m ? `${hour}:${minute}` : <span className="dtp-placeholder">{t('picker.pick_time')}</span>}
            </Trigger>
            {popover.isOpen &&
                ReactDOM.createPortal(
                    <div
                        {...popover.panelProps}
                        className={clsx(popover.panelProps.className, 'dtp-time')}
                        id={panelId}
                        role="dialog"
                        aria-label={ariaLabel}
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
                                            popover.close();
                                        }}
                                    >
                                        {mm}
                                    </button>
                                ))}
                            </div>
                        </div>
                    </div>,
                    document.body,
                )}
        </>
    );
}
