import clsx from 'clsx';
import {
    type CSSProperties,
    type PointerEvent as ReactPointerEvent,
    type Ref,
    useCallback,
    useEffect,
    useLayoutEffect,
    useMemo,
    useRef,
    useState,
} from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import { useSettingsStore } from '../../stores/useSettingsStore';
import type { Server } from '../../stores/useServerStore';
import type { SystemMonitorMode } from '../../types/daemon';
import { Icon } from '../Icon';
import { DEFAULT_MONITOR_FLOAT, floatFromPoint, placeFloat } from './floatPosition';
import { MetricRing, MiniRing, MonitorPicker } from './MonitorRing';
import {
    buildRings,
    colorSlots,
    defaultItems,
    formatRate,
    IDLE_COLOR,
    type MetricsSnapshot,
    nextPeaks,
    OTHERS_COLOR,
    type Peaks,
    pickerGroups,
    type PortRow,
    portRows,
    wantsGpu,
} from './systemMetrics';
import './SystemMonitor.css';

/** 펼쳤을 때와 접었을 때(요약 한 줄)의 표본 간격 */
const OPEN_INTERVAL_MS = 2000;
const SUMMARY_INTERVAL_MS = 5000;

export const systemMonitorAvailable = () => !!window.api?.systemMetrics;

/** 이 너비 이상이면 고정 모니터를 상세 오른쪽 열로, 아니면 상세 위에 둔다 */
export const MONITOR_SIDE_MIN_WIDTH = 1500;
/** 떠 있는 창을 놓을 판이 이보다 좁으면 요약 알약을 한 줄에 하나씩 */
const FLOAT_TIGHT_WIDTH = 320;
/** 원 크기: 떠 있는 창은 내용을 덜 가리게 조금 작게 */
const RING_SIZE = { top: 92, side: 100, float: 84 } as const;

/** 서버 페이지에서 모니터를 둘 자리 (사용자 11-11: 끔 · 고정 · 떠 있는 창) */
export type MonitorPlacement = 'none' | 'top' | 'side' | 'float';
export function monitorPlacement(mode: SystemMonitorMode, expanded: boolean, width: number): MonitorPlacement {
    if (mode === 'off') return 'none';
    if (mode === 'floating') return 'float';
    return expanded && width >= MONITOR_SIDE_MIN_WIDTH ? 'side' : 'top';
}

/** 앱을 켠 뒤 본 디스크 IO·네트워크 최고값 (화면을 오가도 이어진다). 데몬마다 따로 둔다: 다른 PC의 최고값이 눈금이 되지 않게 */
const noPeaks: Peaks = { diskIo: 0, network: 0 };
const sessionPeaks = new Map<string, Peaks>();
let lastTarget = '';

/** 지금 다룰 데몬 키 (이 PC는 ''). 브리지가 없거나 실패하면 이 PC로 본다 */
async function currentTarget(): Promise<string> {
    try {
        const res = await window.api?.daemonTarget?.();
        return res?.target || '';
    } catch {
        return '';
    }
}

/** 데몬 표본을 주기적으로 받는다. 창이 가려져 있으면 쉰다. 요약만 볼 때는 폴더 크기를 재지 않게 한다. GPU는 고른 때만 묻는다 */
export function useSystemMetrics(intervalMs: number, sizes = true, gpu = false) {
    const [state, setState] = useState<{ snapshot: MetricsSnapshot | null; peaks: Peaks }>({
        snapshot: null,
        peaks: sessionPeaks.get(lastTarget) ?? noPeaks,
    });
    const [error, setError] = useState(false);

    useEffect(() => {
        if (!systemMonitorAvailable()) return undefined;
        let alive = true;
        let timer: ReturnType<typeof setTimeout> | null = null;
        const tick = async () => {
            if (!document.hidden) {
                try {
                    const target = await currentTarget();
                    const snap = (await window.api.systemMetrics({ sizes, gpu })) as MetricsSnapshot;
                    if (!alive) return;
                    if (snap?.cpu) {
                        lastTarget = target;
                        const peaks = nextPeaks(sessionPeaks.get(target) ?? noPeaks, snap);
                        sessionPeaks.set(target, peaks);
                        setState({ snapshot: snap, peaks });
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
    }, [intervalMs, sizes, gpu]);

    return { ...state, error };
}

/** 떠 있는 창(FloatingMonitor)이 모니터 판에 거는 자리·끌기 */
interface FloatBindings {
    ref: Ref<HTMLElement>;
    style: CSSProperties;
    dragging: boolean;
    tight: boolean;
    onPointerDown: (e: ReactPointerEvent<HTMLElement>) => void;
    onPointerMove: (e: ReactPointerEvent<HTMLElement>) => void;
    onPointerUp: (e: ReactPointerEvent<HTMLElement>) => void;
}

/**
 * 서버 페이지의 시스템 모니터.
 * - layout="side": 넓은 창의 오른쪽 열 (원형 그래프 세로)
 * - layout="top": 상세 위 (펼치면 원형 그래프 가로, 접으면 요약 한 줄)
 * - layout="float": 앱 안에서 끌어 옮기는 떠 있는 창 (펼치면 원형 그래프 두 열, 접으면 요약 두 줄 알약)
 */
interface SystemMonitorProps {
    layout?: 'top' | 'side' | 'float';
    servers?: Pick<Server, 'id' | 'name'>[];
    float?: FloatBindings;
}

/** 포트별 받기·보내기 (고급 옵션 포트·서버별 네트워크를 셀 때만) */
function PortTable({ rows, t }: { rows: PortRow[]; t: TFunction }) {
    return (
        <table className="monitor-ports">
            <caption>{t('monitor.ports_title')}</caption>
            <thead>
                <tr>
                    <th scope="col">{t('monitor.ports_server')}</th>
                    <th scope="col">{t('monitor.ports_port')}</th>
                    <th scope="col">{t('monitor.ports_rx')}</th>
                    <th scope="col">{t('monitor.ports_tx')}</th>
                </tr>
            </thead>
            <tbody>
                {rows.map((r) => (
                    <tr key={`${r.instance_id}:${r.field}:${r.port}:${r.protocol}`}>
                        <td>
                            <span className="monitor-port-server">
                                <span className="monitor-swatch" style={{ background: r.color }} />
                                {r.name}
                            </span>
                        </td>
                        <td data-tooltip={r.field}>
                            {r.port} <span className="muted">{r.protocol.toUpperCase()}</span>
                        </td>
                        <td>{formatRate(r.rx_bps)}</td>
                        <td>{formatRate(r.tx_bps)}</td>
                    </tr>
                ))}
            </tbody>
        </table>
    );
}

export function SystemMonitor({ layout = 'top', servers = [], float }: SystemMonitorProps) {
    const { t } = useTranslation('gui');
    const open = useSettingsStore((s) => s.systemMonitorExpanded);
    const mode = useSettingsStore((s) => s.systemMonitorMode);
    const picked = useSettingsStore((s) => s.monitorItems);
    const update = useSettingsStore((s) => s.update);
    const [picking, setPicking] = useState(false);
    const { snapshot, error, peaks } = useSystemMetrics(open ? OPEN_INTERVAL_MS : SUMMARY_INTERVAL_MS, open, wantsGpu(picked ?? []));
    const order = useMemo(() => servers.map((s) => s.id), [servers]);
    const slots = useMemo(() => colorSlots(order), [order]);
    const items = useMemo(() => picked ?? defaultItems(snapshot), [picked, snapshot]);
    const rings = useMemo(() => buildRings(snapshot, slots, peaks, items), [snapshot, slots, peaks, items]);
    const ports = useMemo(() => (items.includes('network') ? portRows(snapshot, slots) : []), [items, snapshot, slots]);

    if (!systemMonitorAvailable()) return null;

    const toggle = (
        <button
            type="button"
            data-spot="monitor.toggle"
            className="icon-btn monitor-toggle"
            onClick={() => update({ systemMonitorExpanded: !open })}
            aria-expanded={open}
            aria-label={open ? t('monitor.collapse') : t('monitor.expand')}
            data-tooltip={open ? t('monitor.collapse') : t('monitor.expand')}
        >
            <Icon name={open ? (layout === 'side' ? 'chevronRight' : 'chevronUp') : 'chevronDown'} size="sm" />
        </button>
    );

    // 끄기: 되돌리기가 있는 알림을 띄운다. 다시 켜는 곳은 설정 → 일반
    const hide = (
        <button
            type="button"
            className="icon-btn monitor-hide"
            onClick={() => {
                const prev = mode === 'off' ? 'docked' : mode;
                update({ systemMonitorMode: 'off' });
                window.showToast?.(t('monitor.hidden_toast'), 'info', 6000, {
                    action: {
                        label: t('monitor.undo'),
                        onClick: () => useSettingsStore.getState().update({ systemMonitorMode: prev }),
                    },
                });
            }}
            aria-label={t('monitor.hide')}
            data-tooltip={t('monitor.hide')}
        >
            <Icon name="eyeOff" size="sm" />
        </button>
    );
    const grip = float && (
        <span className="monitor-grip" aria-hidden="true">
            <Icon name="grip" size="sm" />
        </span>
    );
    // 떠 있는 창이면 판 자체가 끌리고 자리를 가진다
    const floatProps = float && {
        ref: float.ref as Ref<HTMLElement>,
        style: float.style,
        onPointerDown: float.onPointerDown,
        onPointerMove: float.onPointerMove,
        onPointerUp: float.onPointerUp,
        onPointerCancel: float.onPointerUp,
    };
    const floatClass = float && clsx('monitor-float', { dragging: float.dragging, tight: float.tight });

    if (!open) {
        return (
            <section
                data-spot="monitor"
                className={clsx('monitor monitor-summary', floatClass)}
                aria-label={t('monitor.title')}
                {...floatProps}
            >
                {grip}
                {!float && (
                    <>
                        <Icon name="monitor" size="sm" />
                        <span className="monitor-summary-title">{t('monitor.title')}</span>
                    </>
                )}
                <div className={clsx('monitor-chips', float && 'grid')}>
                    {rings.map((r) => (
                        <span key={r.key} className="monitor-chip">
                            <MiniRing ring={r} />
                            <span className="muted">{t(r.titleKey, r.titleVars ?? {})}</span>{' '}
                            <strong>
                                {r.value}
                                {r.unit}
                            </strong>
                        </span>
                    ))}
                    {!snapshot && <span className="muted">{error ? t('monitor.unavailable') : t('monitor.loading')}</span>}
                </div>
                <span className="monitor-actions">
                    {hide}
                    {toggle}
                </span>
            </section>
        );
    }

    // 범례: 이번 표본에 값이 있는 인스턴스만, 서버 목록 순서로
    const shown = new Set(rings.flatMap((r) => r.segments.map((s) => s.key)));
    const legend = servers.filter((s) => shown.has(s.id) && slots[s.id]);
    const hasMore = shown.has('__more');

    return (
        <section
            data-spot="monitor"
            className={clsx('monitor', !float && `monitor-${layout}`, floatClass)}
            aria-label={t('monitor.title')}
            {...floatProps}
        >
            <header className="monitor-head">
                {grip ?? <Icon name="monitor" size="sm" />}
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
                {hide}
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
                                <MetricRing key={r.key} ring={r} t={t} size={RING_SIZE[layout]} inlineNote={false} />
                            ))}
                        </div>
                    )}
                    {ports.length > 0 && <PortTable rows={ports} t={t} />}
                    {layout === 'side' && <p className="monitor-footnote">{t('monitor.footnote')}</p>}
                </>
            )}
        </section>
    );
}

/** 끌기를 시작하지 않는 곳 (버튼·입력·고르기 창·표) */
const NO_DRAG = 'button, input, label, a, select, .monitor-pick-panel, .monitor-ports';

/**
 * 떠 있는 시스템 모니터 (사용자 11-11). 서버 페이지 위에 겹쳐 놓고, 끌어 옮기면 가까운 모서리 기준 자리를 설정에 남긴다.
 * 모서리 가까이 놓으면 붙고, 창이 줄어 밖으로 밀리면 안쪽으로 돌아온다 (floatPosition.ts)
 */
export function FloatingMonitor({ servers = [] }: { servers?: Pick<Server, 'id' | 'name'>[] }) {
    const saved = useSettingsStore((s) => s.systemMonitorFloat) ?? DEFAULT_MONITOR_FLOAT;
    const open = useSettingsStore((s) => s.systemMonitorExpanded);
    const update = useSettingsStore((s) => s.update);
    const layerRef = useRef<HTMLDivElement>(null);
    const boxRef = useRef<HTMLElement>(null);
    const [size, setSize] = useState({ aw: 0, ah: 0, bw: 0, bh: 0 });
    const [drag, setDrag] = useState<{ left: number; top: number } | null>(null);
    const start = useRef<{ x: number; y: number; left: number; top: number; id: number } | null>(null);

    // 판과 모니터 크기는 그때그때 DOM에서 읽는다 (원 개수·접힘·창 크기에 따라 바뀐다)
    const read = useCallback(() => {
        const layer = layerRef.current;
        const box = boxRef.current;
        return {
            area: { width: layer?.clientWidth ?? 0, height: layer?.clientHeight ?? 0 },
            box: { width: box?.offsetWidth ?? 0, height: box?.offsetHeight ?? 0 },
        };
    }, []);
    const measure = useCallback(() => {
        const { area, box } = read();
        setSize((prev) =>
            prev.aw === area.width && prev.ah === area.height && prev.bw === box.width && prev.bh === box.height
                ? prev
                : { aw: area.width, ah: area.height, bw: box.width, bh: box.height },
        );
    }, [read]);
    // biome-ignore lint/correctness/useExhaustiveDependencies: open 은 접고 펼 때 바뀌는 모니터 판을 다시 지켜보라는 신호
    useLayoutEffect(() => {
        measure();
        if (typeof ResizeObserver === 'undefined') return undefined;
        const ro = new ResizeObserver(measure);
        if (layerRef.current) ro.observe(layerRef.current);
        if (boxRef.current) ro.observe(boxRef.current);
        return () => ro.disconnect();
    }, [measure, open]);

    const measured = size.aw > 0 && size.bw > 0;
    const at = drag ?? placeFloat(saved, { width: size.aw, height: size.ah }, { width: size.bw, height: size.bh });

    const onPointerDown = (e: ReactPointerEvent<HTMLElement>) => {
        if (e.button !== 0 || (e.target as Element).closest(NO_DRAG)) return;
        // 판은 user-select: none 이어도 끌면서 아래 콘솔 글자까지 선택이 번지므로 기본 동작을 막는다
        e.preventDefault();
        const { area, box } = read();
        const from = placeFloat(saved, area, box);
        start.current = { x: e.clientX, y: e.clientY, left: from.left, top: from.top, id: e.pointerId };
        e.currentTarget.setPointerCapture?.(e.pointerId);
        setDrag(from);
    };
    const onPointerMove = (e: ReactPointerEvent<HTMLElement>) => {
        const s = start.current;
        if (!s || s.id !== e.pointerId) return;
        const { area, box } = read();
        setDrag({
            left: Math.max(0, Math.min(area.width - box.width, s.left + e.clientX - s.x)),
            top: Math.max(0, Math.min(area.height - box.height, s.top + e.clientY - s.y)),
        });
    };
    const onPointerUp = (e: ReactPointerEvent<HTMLElement>) => {
        const s = start.current;
        if (!s || s.id !== e.pointerId) return;
        start.current = null;
        const { area, box } = read();
        const left = Math.max(0, Math.min(area.width - box.width, s.left + e.clientX - s.x));
        const top = Math.max(0, Math.min(area.height - box.height, s.top + e.clientY - s.y));
        setDrag(null);
        if (left !== s.left || top !== s.top) update({ systemMonitorFloat: floatFromPoint(left, top, area, box) });
    };

    return (
        <div ref={layerRef} className="monitor-float-layer">
            <SystemMonitor
                layout="float"
                servers={servers}
                float={{
                    ref: boxRef,
                    // 처음 크기를 재기 전에는 왼쪽 위에 번쩍이지 않게 숨긴다
                    style: { left: at.left, top: at.top, visibility: measured ? undefined : 'hidden' },
                    dragging: drag !== null,
                    tight: size.aw > 0 && size.aw < FLOAT_TIGHT_WIDTH,
                    onPointerDown,
                    onPointerMove,
                    onPointerUp,
                }}
            />
        </div>
    );
}
