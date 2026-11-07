import type { TFunction } from 'i18next';
import { SabaCheckbox } from '../ui/SabaUI';
import { IDLE_COLOR, type PickerGroup, type Ring, type RingSegment, toggleItem } from './systemMetrics';
import './MonitorRing.css';

/*
 * 시스템 모니터 원 하나와 항목 고르기 창. 스토어를 모른다: GUI 서버 페이지와 웹 콘솔이 같이 쓴다.
 */

const RADIUS = 42;
const STROKE = 9;
const CIRC = 2 * Math.PI * RADIUS;
/** 원은 아래가 트인 270° 호다 */
const ARC = (CIRC * 270) / 360;
/** 조각 사이 틈 (호 길이) */
const GAP = 2.5;

/**
 * 서버로 나누지 않는 몫(전체·나머지 프로세스)은 원 종류마다 채도를 크게 낮춘 바탕색으로 칠하고,
 * 서버 몫만 계열색으로 덧칠한다 (사용자 11-07). 바탕색은 MonitorRing.css의 `--ring-base`다.
 */
const BASE_KEYS = new Set(['__all', '__others']);

/**
 * 호 [from, to]를 둥근 끝 선으로 그릴 dash 값 (바탕 고리). 둥근 끝이 양쪽으로 STROKE/2씩 나가므로 그만큼 줄여 그린다.
 */
function arcDash(from: number, to: number) {
    const start = from + STROKE / 2;
    const len = Math.max(0.01, to - from - STROKE);
    return { strokeDasharray: `${len} ${CIRC}`, strokeDashoffset: -start };
}

/** 색 조각은 각진 끝으로 [from, to]를 그대로 그린다: 작은 몫이 점으로 부풀지 않게 */
function flatDash(from: number, to: number) {
    return { strokeDasharray: `${Math.max(0, to - from)} ${CIRC}`, strokeDashoffset: -from };
}

/** 호 위 길이 at의 점 (회전 전 좌표) */
function pointAt(at: number) {
    const a = at / RADIUS;
    return { cx: 50 + RADIUS * Math.cos(a), cy: 50 + RADIUS * Math.sin(a) };
}

interface MetricRingProps {
    ring: Ring;
    t: TFunction;
    size?: number;
    /** 안내 문구를 원 아래에 쓸지, 원에 말풍선으로 달지 */
    inlineNote?: boolean;
}

/** 원형 그래프 하나. 조각마다 말풍선으로 이름과 값을 보여 준다 */
export function MetricRing({ ring, t, size = 104, inlineNote = true }: MetricRingProps) {
    const total = ring.total > 0 ? ring.total : 1;
    const visible = ring.segments.filter((s) => s.value > 0 && s.color !== IDLE_COLOR);
    // 채운 부분 전체의 양 끝만 둥글다: 끝점 반원(점)이 STROKE/2를 덮으므로 조각은 그 안쪽에서 시작·끝난다
    const half = STROKE / 2;
    const filled = Math.min(ARC, visible.reduce((sum, s) => sum + (s.value / total) * ARC, 0));
    let offset = 0;
    const arcs = visible.map((seg, i) => {
        const len = Math.min(ARC - offset, (seg.value / total) * ARC);
        const from = Math.max(half, offset);
        offset += len;
        const last = i === visible.length - 1;
        const to = Math.min(filled - half, last ? offset : offset - GAP);
        return { seg, from, to: Math.max(from, to) };
    });
    const firstColor = arcs[0]?.seg;
    const lastColor = arcs[arcs.length - 1]?.seg;
    const stroke = (seg: RingSegment) => (BASE_KEYS.has(seg.key) ? 'var(--ring-base)' : seg.color);
    const label = (seg: RingSegment) => (seg.labelKey ? t(seg.labelKey) : seg.label);
    const title = t(ring.titleKey, ring.titleVars);

    return (
        <figure
            className="monitor-ring"
            data-tone={ring.tone}
            data-metric={ring.key}
            data-tooltip={ring.noteKey && !inlineNote ? t(ring.noteKey) : undefined}
        >
            <figcaption className="monitor-ring-title">{title}</figcaption>
            <div className="monitor-ring-chart" style={{ width: size, height: size }}>
                <svg viewBox="0 0 100 100" role="img" aria-label={`${title} ${ring.value}${ring.unit}`}>
                    <g transform="rotate(135 50 50)">
                        <circle className="monitor-track" cx="50" cy="50" r={RADIUS} strokeWidth={STROKE} style={arcDash(0, ARC)} />
                        {arcs.map(({ seg, from, to }) => (
                            <circle
                                key={seg.key}
                                className="monitor-arc"
                                cx="50"
                                cy="50"
                                r={RADIUS}
                                strokeWidth={STROKE}
                                style={{ ...flatDash(from, to), stroke: stroke(seg) }}
                                data-tooltip={`${label(seg)}: ${ring.format(seg.value)}`}
                            />
                        ))}
                        {firstColor && lastColor && filled > 0 && (
                            <>
                                <circle className="monitor-cap" r={half} {...pointAt(half)} style={{ fill: stroke(firstColor) }} />
                                <circle
                                    className="monitor-cap"
                                    r={half}
                                    {...pointAt(Math.max(half, filled - half))}
                                    style={{ fill: stroke(lastColor) }}
                                />
                            </>
                        )}
                    </g>
                </svg>
                <div className="monitor-ring-center">
                    <strong className={ring.value.length > 3 ? 'long' : undefined}>{ring.value}</strong>
                    {ring.unit && <small>{ring.unit}</small>}
                </div>
            </div>
            {ring.pills.length > 0 && (
                <div className="monitor-pills">
                    {ring.pills.map((p) => (
                        <span key={p.key ?? p.text} className={p.unknown ? 'monitor-pill unknown' : 'monitor-pill'}>
                            {p.key ? t(p.key, p.vars) : p.text}
                        </span>
                    ))}
                </div>
            )}
            {ring.noteKey && inlineNote && <div className="monitor-ring-note">{t(ring.noteKey)}</div>}
        </figure>
    );
}

interface MonitorPickerProps {
    groups: PickerGroup[];
    items: string[];
    onChange: (items: string[]) => void;
    t: TFunction;
}

/** 시스템 모니터에 띄울 항목 고르기. 붙은 원을 끄면 그 원의 보조 값 줄은 고를 수 없다 */
export function MonitorPicker({ groups, items, onChange, t }: MonitorPickerProps) {
    const on = new Set(items);
    return (
        <div className="monitor-picker">
            {groups.map((g) => (
                <fieldset key={g.key} className="monitor-picker-group">
                    <legend>{t(g.titleKey)}</legend>
                    {g.rows.length === 0 && g.emptyKey && <p className="monitor-picker-empty">{t(g.emptyKey)}</p>}
                    {g.rows.map((r) => (
                        <SabaCheckbox
                            key={r.id}
                            checked={on.has(r.id)}
                            disabled={!!r.parent && !on.has(r.parent)}
                            onChange={(checked) => onChange(toggleItem(items, r.id, checked))}
                            data-item={r.id}
                        >
                            <span className="monitor-picker-label">{r.labelKey ? t(r.labelKey, r.labelVars) : r.label}</span>
                            {r.noteKey && <small className="monitor-picker-note">{t(r.noteKey)}</small>}
                        </SabaCheckbox>
                    ))}
                </fieldset>
            ))}
        </div>
    );
}

/** 요약 줄의 작은 원 (사용자 11-07): 큰 원과 같은 270° 호와 색 조각, 글자·말풍선 없이 */
export function MiniRing({ ring, size = 20 }: { ring: Ring; size?: number }) {
    const total = ring.total > 0 ? ring.total : 1;
    const segs = ring.segments.filter((s) => s.value > 0 && s.color !== IDLE_COLOR);
    // pathLength 100인 원 위에서 270°(75)만 쓴다. 시작은 왼쪽 아래(135° 돌림)
    let at = 0;
    return (
        <svg className="monitor-mini" data-tone={ring.tone} viewBox="0 0 24 24" width={size} height={size} aria-hidden="true">
            <g transform="rotate(135 12 12)">
                <circle className="monitor-track" cx="12" cy="12" r="9" strokeWidth={4} pathLength={100} strokeDasharray="75 100" />
                {segs.map((seg) => {
                    const len = Math.min(75 - at, (seg.value / total) * 75);
                    const from = at;
                    at += len;
                    return (
                        <circle
                            key={seg.key}
                            className="monitor-mini-arc"
                            cx="12"
                            cy="12"
                            r="9"
                            strokeWidth={4}
                            pathLength={100}
                            strokeDasharray={`${len} 100`}
                            strokeDashoffset={-from}
                            style={{ stroke: BASE_KEYS.has(seg.key) ? 'var(--ring-base)' : seg.color }}
                        />
                    );
                })}
            </g>
        </svg>
    );
}
