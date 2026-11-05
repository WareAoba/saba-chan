import clsx from 'clsx';
import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSettingsStore } from '../../stores/useSettingsStore';
import type { Server } from '../../stores/useServerStore';
import { Icon } from '../Icon';
import { MetricRing, MonitorPicker } from './MonitorRing';
import {
    buildRings,
    colorSlots,
    defaultItems,
    IDLE_COLOR,
    type MetricsSnapshot,
    nextPeaks,
    OTHERS_COLOR,
    type Peaks,
    pickerGroups,
} from './systemMetrics';
import './SystemMonitor.css';

/** 펼쳤을 때와 접었을 때(요약 한 줄)의 표본 간격 */
const OPEN_INTERVAL_MS = 2000;
const SUMMARY_INTERVAL_MS = 5000;

export const systemMonitorAvailable = () => !!window.api?.systemMetrics;

/** 앱을 켠 뒤 본 디스크 IO·네트워크 최고값 (화면을 오가도 이어진다) */
let sessionPeaks: Peaks = { diskIo: 0, network: 0 };

/** 데몬 표본을 주기적으로 받는다. 창이 가려져 있으면 쉰다. 요약만 볼 때는 폴더 크기를 재지 않게 한다. */
export function useSystemMetrics(intervalMs: number, sizes = true) {
    const [state, setState] = useState<{ snapshot: MetricsSnapshot | null; peaks: Peaks }>({
        snapshot: null,
        peaks: sessionPeaks,
    });
    const [error, setError] = useState(false);

    useEffect(() => {
        if (!systemMonitorAvailable()) return undefined;
        let alive = true;
        let timer: ReturnType<typeof setTimeout> | null = null;
        const tick = async () => {
            if (!document.hidden) {
                try {
                    const snap = (await window.api.systemMetrics({ sizes })) as MetricsSnapshot;
                    if (!alive) return;
                    if (snap?.cpu) {
                        sessionPeaks = nextPeaks(sessionPeaks, snap);
                        setState({ snapshot: snap, peaks: sessionPeaks });
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
            clearTimeout(timer as ReturnType<typeof setTimeout>);
        };
    }, [intervalMs, sizes]);

    return { ...state, error };
}

/**
 * 서버 페이지의 시스템 모니터.
 * - layout="side": 넓은 창의 오른쪽 열 (원형 그래프 세로)
 * - layout="top": 상세 위 (펼치면 원형 그래프 가로, 접으면 요약 한 줄)
 */
interface SystemMonitorProps {
    layout?: 'top' | 'side';
    servers?: Pick<Server, 'id' | 'name'>[];
}

export function SystemMonitor({ layout = 'top', servers = [] }: SystemMonitorProps) {
    const { t } = useTranslation('gui');
    const open = useSettingsStore((s) => s.systemMonitorOpen);
    const picked = useSettingsStore((s) => s.monitorItems);
    const update = useSettingsStore((s) => s.update);
    const [picking, setPicking] = useState(false);
    const { snapshot, error, peaks } = useSystemMetrics(open ? OPEN_INTERVAL_MS : SUMMARY_INTERVAL_MS, open);
    const order = useMemo(() => servers.map((s) => s.id), [servers]);
    const slots = useMemo(() => colorSlots(order), [order]);
    const items = useMemo(() => picked ?? defaultItems(snapshot), [picked, snapshot]);
    const rings = useMemo(() => buildRings(snapshot, slots, peaks, items), [snapshot, slots, peaks, items]);

    if (!systemMonitorAvailable()) return null;

    const toggle = (
        <button
            type="button"
            className="icon-btn monitor-toggle"
            onClick={() => update({ systemMonitorOpen: !open })}
            aria-expanded={open}
            aria-label={open ? t('monitor.collapse') : t('monitor.expand')}
            data-tooltip={open ? t('monitor.collapse') : t('monitor.expand')}
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
                            <span className="muted">{t(r.titleKey, r.titleVars)}</span>{' '}
                            <strong>
                                {r.value}
                                {r.unit}
                            </strong>
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
                {snapshot && (
                    <button
                        type="button"
                        className={clsx('icon-btn monitor-pick-toggle', picking && 'active')}
                        onClick={() => setPicking((v) => !v)}
                        aria-expanded={picking}
                        aria-label={t('monitor.pick')}
                        data-tooltip={t('monitor.pick')}
                    >
                        <Icon name="settings" size="sm" />
                    </button>
                )}
                {toggle}
            </header>
            {!snapshot ? (
                <p className="muted monitor-empty">{error ? t('monitor.unavailable') : t('monitor.loading')}</p>
            ) : (
                <>
                    {picking && (
                        <div className="monitor-pick-panel">
                            <MonitorPicker groups={pickerGroups(snapshot)} items={items} onChange={(next) => update({ monitorItems: next })} t={t} />
                            {picked && (
                                <button type="button" className="btn btn-sm btn-ghost" onClick={() => update({ monitorItems: null })}>
                                    {t('monitor.pick_reset')}
                                </button>
                            )}
                        </div>
                    )}
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
                    {rings.length === 0 ? (
                        <p className="muted monitor-empty">{t('monitor.nothing_picked')}</p>
                    ) : (
                        <div className="monitor-rings">
                            {rings.map((r) => (
                                <MetricRing key={r.key} ring={r} t={t} size={layout === 'side' ? 100 : 92} inlineNote={false} />
                            ))}
                        </div>
                    )}
                    {layout === 'side' && <p className="monitor-footnote">{t('monitor.footnote')}</p>}
                </>
            )}
        </section>
    );
}
