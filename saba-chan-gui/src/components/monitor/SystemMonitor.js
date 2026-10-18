import clsx from 'clsx';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSettingsStore } from '../../stores/useSettingsStore';
import { Icon } from '../Icon';
import { buildRings, colorSlots, IDLE_COLOR, OTHERS_COLOR, PEAK_WINDOW, peaksOf } from './systemMetrics';
import './SystemMonitor.css';

/** 펼쳤을 때와 접었을 때(요약 한 줄)의 표본 간격 */
const OPEN_INTERVAL_MS = 2000;
const SUMMARY_INTERVAL_MS = 5000;

export const systemMonitorAvailable = () => !!window.api?.systemMetrics;

/** 데몬 표본을 주기적으로 받는다. 창이 가려져 있으면 쉰다. 요약만 볼 때는 폴더 크기를 재지 않게 한다. */
export function useSystemMetrics(intervalMs, sizes = true) {
    const [state, setState] = useState({ snapshot: null, peaks: peaksOf([]) });
    const [error, setError] = useState(false);
    const history = useRef([]);

    useEffect(() => {
        if (!systemMonitorAvailable()) return undefined;
        let alive = true;
        let timer = null;
        const tick = async () => {
            if (!document.hidden) {
                try {
                    const snap = await window.api.systemMetrics({ sizes });
                    if (!alive) return;
                    if (snap && snap.cpu) {
                        history.current = [...history.current, snap].slice(-PEAK_WINDOW);
                        setState({ snapshot: snap, peaks: peaksOf(history.current) });
                        setError(false);
                    } else {
                        setError(true);
                    }
                } catch {
                    if (alive) setError(true);
                }
            }
            if (alive) timer = setTimeout(tick, intervalMs);
        };
        tick();
        return () => {
            alive = false;
            clearTimeout(timer);
        };
    }, [intervalMs, sizes]);

    return { ...state, error };
}

const RADIUS = 42;
const STROKE = 12;
const CIRC = 2 * Math.PI * RADIUS;
/** 조각 사이 틈 (원주 길이) */
const GAP = 2;

/** 원형 그래프 하나. 조각마다 <title>로 이름과 값을 보여 준다 */
export function MetricRing({ ring, t, size = 112, inlineNote = true }) {
    const total = ring.total > 0 ? ring.total : 1;
    const visible = ring.segments.filter((s) => s.value > 0);
    let offset = 0;
    const arcs = visible.map((seg) => {
        const len = Math.min(CIRC, (seg.value / total) * CIRC);
        const gap = visible.length > 1 && len > GAP * 2 ? GAP : 0;
        const arc = { seg, dash: Math.max(0, len - gap), offset };
        offset += len;
        return arc;
    });
    const label = (seg) => (seg.labelKey ? t(seg.labelKey) : seg.label);
    const sub = ring.sub?.key ? t(ring.sub.key, ring.sub.vars) : ring.sub?.text;

    return (
        <figure className="monitor-ring" data-metric={ring.key} title={ring.noteKey && !inlineNote ? t(ring.noteKey) : undefined}>
            <figcaption className="monitor-ring-title">{t(ring.titleKey)}</figcaption>
            <div className="monitor-ring-chart" style={{ width: size, height: size }}>
                <svg viewBox="0 0 100 100" role="img" aria-label={`${t(ring.titleKey)} ${ring.center}`}>
                    <circle cx="50" cy="50" r={RADIUS} fill="none" stroke={IDLE_COLOR} strokeWidth={STROKE} />
                    {arcs.map(({ seg, dash, offset: off }) =>
                        seg.color === IDLE_COLOR ? null : (
                            <circle
                                key={seg.key}
                                className="monitor-arc"
                                cx="50"
                                cy="50"
                                r={RADIUS}
                                fill="none"
                                stroke={seg.color}
                                strokeWidth={STROKE}
                                strokeDasharray={`${dash} ${CIRC - dash}`}
                                strokeDashoffset={-off}
                                transform="rotate(-90 50 50)"
                            >
                                <title>{`${label(seg)}: ${ring.format(seg.value)}`}</title>
                            </circle>
                        ),
                    )}
                </svg>
                <div className="monitor-ring-center">
                    <strong>{ring.center}</strong>
                </div>
            </div>
            {sub && (
                // 좁을 때는 "·" 뒤에서만 줄을 바꾼다 ("· 쓰기"가 다음 줄 맨 앞에 오지 않게): 조각 안의 띄어쓰기는 붙인다
                <div className="monitor-ring-sub">
                    {String(sub)
                        .split(' · ')
                        .map((part) => part.replaceAll(' ', '\u00a0'))
                        .join('\u00a0· ')}
                </div>
            )}
            {ring.noteKey && inlineNote && <div className="monitor-ring-note">{t(ring.noteKey)}</div>}
        </figure>
    );
}

/**
 * 서버 페이지의 시스템 모니터.
 * - layout="side": 넓은 창의 오른쪽 열 (원형 그래프 세로)
 * - layout="top": 상세 위 (펼치면 원형 그래프 가로, 접으면 요약 한 줄)
 */
export function SystemMonitor({ layout = 'top', servers = [] }) {
    const { t } = useTranslation('gui');
    const open = useSettingsStore((s) => s.systemMonitorOpen);
    const update = useSettingsStore((s) => s.update);
    const { snapshot, error, peaks } = useSystemMetrics(open ? OPEN_INTERVAL_MS : SUMMARY_INTERVAL_MS, open);
    const order = useMemo(() => servers.map((s) => s.id), [servers]);
    const slots = useMemo(() => colorSlots(order), [order]);
    const rings = useMemo(() => buildRings(snapshot, slots, peaks), [snapshot, slots, peaks]);

    if (!systemMonitorAvailable()) return null;

    const toggle = (
        <button
            type="button"
            className="icon-btn monitor-toggle"
            onClick={() => update({ systemMonitorOpen: !open })}
            aria-expanded={open}
            aria-label={open ? t('monitor.collapse') : t('monitor.expand')}
            title={open ? t('monitor.collapse') : t('monitor.expand')}
        >
            <Icon name={open ? (layout === 'side' ? 'chevronRight' : 'chevronUp') : 'chevronDown'} size="sm" />
        </button>
    );

    if (!open) {
        return (
            <section className="monitor monitor-summary" aria-label={t('monitor.title')}>
                <Icon name="monitor" size="sm" />
                <span className="monitor-summary-title">{t('monitor.title')}</span>
                <div className="monitor-chips">
                    {rings.map((r) => (
                        <span key={r.key} className="monitor-chip">
                            <span className="muted">{t(r.titleKey)}</span> <strong>{r.center}</strong>
                        </span>
                    ))}
                    {!snapshot && <span className="muted">{error ? t('monitor.unavailable') : t('monitor.loading')}</span>}
                </div>
                {toggle}
            </section>
        );
    }

    // 범례: 이번 표본에 값이 있는 인스턴스만, 서버 목록 순서로
    const shown = new Set(rings.flatMap((r) => r.segments.map((s) => s.key)));
    const legend = servers.filter((s) => shown.has(s.id) && slots[s.id]);
    const hasMore = shown.has('__more');

    return (
        <section className={clsx('monitor', `monitor-${layout}`)} aria-label={t('monitor.title')}>
            <header className="monitor-head">
                <Icon name="monitor" size="sm" />
                <h3>{t('monitor.title')}</h3>
                {toggle}
            </header>
            {!snapshot ? (
                <p className="muted monitor-empty">{error ? t('monitor.unavailable') : t('monitor.loading')}</p>
            ) : (
                <>
                    <ul className="monitor-legend">
                        {legend.map((s) => (
                            <li key={s.id}>
                                <span className="monitor-swatch" style={{ background: `var(--series-${slots[s.id]})` }} />
                                {s.name}
                            </li>
                        ))}
                        {hasMore && (
                            <li>
                                <span className="monitor-swatch" style={{ background: 'var(--text-2)' }} />
                                {t('monitor.more_instances')}
                            </li>
                        )}
                        <li>
                            <span className="monitor-swatch" style={{ background: OTHERS_COLOR }} />
                            {t('monitor.others')}
                        </li>
                        <li>
                            <span className="monitor-swatch idle" style={{ background: IDLE_COLOR }} />
                            {t('monitor.idle_legend')}
                        </li>
                    </ul>
                    <div className="monitor-rings">
                        {rings.map((r) => (
                            <MetricRing key={r.key} ring={r} t={t} size={layout === 'side' ? 96 : 88} inlineNote={false} />
                        ))}
                    </div>
                    {layout === 'side' && <p className="monitor-footnote">{t('monitor.footnote')}</p>}
                </>
            )}
        </section>
    );
}
