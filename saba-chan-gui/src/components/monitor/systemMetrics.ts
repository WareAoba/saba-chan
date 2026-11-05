/**
 * 시스템 모니터 표시 계산. 값은 데몬 `/api/system/metrics`가 준 그대로 쓰고,
 * 여기서는 고른 항목(`monitorItems`)에 맞춰 원형 그래프 조각(인스턴스별 - 나머지 프로세스 - 유휴)과
 * 보조 값 알약(클럭·온도·RPM 등)을 만들기만 한다.
 */

/** 계열색 개수 (tokens.css --series-1..8). 넘치는 인스턴스는 "기타 인스턴스"로 묶는다 */
export const SERIES_COUNT = 8;
/** 디스크 IO·네트워크 눈금의 최소값: 거의 놀 때 작은 값이 꽉 찬 원으로 보이지 않게 */
const MIN_RATE_SCALE = 1024 * 1024;

/** GET /api/system/metrics 표본 (값은 데몬이 준 그대로) */
export interface MetricsSnapshot {
    cpu?: {
        total_percent?: number;
        cores?: number;
        others_percent?: number;
        clock_mhz?: number | null;
        temp_c?: number | null;
    };
    memory?: { total_bytes?: number; used_bytes?: number; others_bytes?: number; clock_mhz?: number | null };
    disk_io?: { read_bps?: number; write_bps?: number; source?: string };
    network?: { rx_bps?: number; tx_bps?: number; per_instance?: boolean };
    disks?: {
        mount: string;
        total_bytes: number;
        available_bytes: number;
        others_bytes?: number;
        sizes_at_ms?: number | null;
        instances: { id: string; bytes: number }[];
    }[];
    /** 모든 디스크 (마운트 하나에 하나) */
    volumes?: {
        mount: string;
        name?: string;
        kind?: 'ssd' | 'hdd' | 'unknown' | string;
        total_bytes: number;
        available_bytes: number;
        rpm?: number | null;
    }[];
    /** 네트워크 카드별 */
    adapters?: { name: string; rx_bps: number; tx_bps: number }[];
    gpus?: { name?: string | null; util_percent?: number | null; clock_mhz?: number | null; temp_c?: number | null }[];
    instances?: {
        id: string;
        name: string;
        cpu_percent: number;
        memory_bytes: number;
        /** 고급 옵션 포트·서버별 네트워크가 셀 때만 값이 있다 */
        net_rx_bps?: number | null;
        net_tx_bps?: number | null;
    }[];
    /** 포트·서버별 네트워크 (설정 > 고급, Windows ETW). status가 "on"일 때만 값이 있다 */
    net_trace?: NetTrace;
}

/** 데몬 `NetTraceMetrics` */
export interface NetTrace {
    /** off | idle | starting | on | unsupported | needs_grant | needs_relogin | error */
    status: string;
    rx_bps?: number;
    tx_bps?: number;
    others_rx_bps?: number;
    others_tx_bps?: number;
    ports?: PortTraffic[];
}

export interface PortTraffic {
    instance_id: string;
    /** 모듈 설정 필드 이름 (예: port, rcon_port) */
    field: string;
    port: number;
    protocol: string;
    rx_bps: number;
    tx_bps: number;
}

/** 옵션은 켰지만 세지 못하는 까닭: 네트워크 그래프 아래 안내로 보인다 */
const NET_TRACE_NOTES = new Set(['needs_grant', 'needs_relogin', 'error']);

export interface Peaks {
    diskIo: number;
    network: number;
    /** 네트워크 카드 이름 → 최고값 */
    adapters?: Record<string, number>;
}

/** 원형 그래프 조각 */
export interface RingSegment {
    key: string;
    label?: string;
    labelKey?: string;
    value: number;
    color: string;
}

/** 원 아래 보조 값 하나. unknown이면 "모름"으로 흐리게 보인다 */
export interface RingPill {
    key?: string;
    vars?: Record<string, unknown>;
    text?: string;
    unknown?: boolean;
}

/** 원 종류: 색 조각이 아닌 전체 몫(`__all`)의 그라데이션 색을 고른다 */
export type RingTone = 'cpu' | 'memory' | 'gpu' | 'disk' | 'net';

/** 원형 그래프 하나 */
export interface Ring {
    key: string;
    titleKey: string;
    titleVars?: Record<string, unknown>;
    /** 가운데 큰 숫자와 작은 단위 */
    value: string;
    unit: string;
    tone: RingTone;
    pills: RingPill[];
    total: number;
    format: (v: number) => string;
    noteKey?: string | null;
    segments: RingSegment[];
}

/** 바이트 → [숫자, 단위] */
export function splitBytes(n: number): [string, string] {
    if (!Number.isFinite(n) || n < 0) return ['—', ''];
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    let i = 0;
    let v = n;
    while (v >= 1024 && i < units.length - 1) {
        v /= 1024;
        i += 1;
    }
    return [i === 0 ? String(Math.round(v)) : v.toFixed(v < 10 ? 1 : 0), units[i]];
}

export function formatBytes(n: number) {
    const [v, u] = splitBytes(n);
    return u ? `${v} ${u}` : v;
}

export const formatRate = (bps: number) => `${formatBytes(bps)}/s`;
export const formatPercent = (p: number) => (Number.isFinite(p) ? `${Math.round(p)}%` : '—');
const percentValue = (p: number | null | undefined) => (typeof p === 'number' && Number.isFinite(p) ? String(Math.round(p)) : '—');
const formatGhz = (mhz: number) => `${(mhz / 1000).toFixed(2)} GHz`;

/**
 * 인스턴스 id → 계열색 번호. 서버 목록 순서를 따른다: 다른 인스턴스가 꺼져도 색이 바뀌지 않는다.
 * 목록에 없는 id는 null (기타로 묶인다)
 */
export function colorSlots(orderedIds: string[]) {
    const slots: Record<string, number | null> = {};
    orderedIds.forEach((id, i) => {
        slots[id] = i < SERIES_COUNT ? i + 1 : null;
    });
    return slots;
}

const seriesColor = (slot: number | null | undefined) => (slot ? `var(--series-${slot})` : 'var(--text-2)');
export const OTHERS_COLOR = 'var(--text-3)';
export const IDLE_COLOR = 'var(--chart-idle)';
/** 인스턴스로 나누지 않는 전체 몫: 그리는 쪽이 원 종류(tone)의 그라데이션으로 칠한다 */
export const ALL_COLOR = 'var(--text-2)';

/** 인스턴스 값들을 색 조각으로. 계열색 밖 인스턴스는 한 조각("기타 인스턴스")으로 합친다 */
function instanceSegments(
    values: { id: string; value: number }[],
    slots: Record<string, number | null>,
    names: Record<string, string>,
) {
    const segs: RingSegment[] = [];
    let overflow = 0;
    for (const { id, value } of values) {
        if (!(value > 0)) continue;
        const slot = slots[id];
        if (slot) segs.push({ key: id, label: names[id] || id, value, color: seriesColor(slot) });
        else overflow += value;
    }
    segs.sort((a, b) => (slots[a.key] as number) - (slots[b.key] as number));
    if (overflow > 0) segs.push({ key: '__more', labelKey: 'monitor.more_instances', value: overflow, color: seriesColor(null) });
    return segs;
}

/** 디스크 IO·네트워크·네트워크 카드 최대값을 새 표본으로 갱신한다 (앱을 켠 뒤 본 최고값이 눈금이 된다) */
export function nextPeaks(prev: Peaks, snap: MetricsSnapshot | null | undefined): Peaks {
    const adapters = { ...(prev.adapters || {}) };
    for (const a of snap?.adapters || []) {
        adapters[a.name] = Math.max(adapters[a.name] || 0, (a.rx_bps || 0) + (a.tx_bps || 0));
    }
    return {
        diskIo: Math.max(prev.diskIo, (snap?.disk_io?.read_bps || 0) + (snap?.disk_io?.write_bps || 0)),
        // 서버별로 셀 때는 ETW 합(루프백 포함)이 네트워크 카드 값보다 클 수 있다: 둘 중 큰 쪽
        network: Math.max(
            prev.network,
            (snap?.network?.rx_bps || 0) + (snap?.network?.tx_bps || 0),
            snap?.net_trace?.status === 'on' ? (snap.net_trace.rx_bps || 0) + (snap.net_trace.tx_bps || 0) : 0,
        ),
        adapters,
    };
}

// ── 고를 수 있는 항목 ──────────────────────────────────────────────
// 원 하나가 항목 하나다(cpu, memory, gpu, disk_io, volume:<마운트>, network, adapter:<이름>).
// 점이 붙은 항목(cpu.clock 등)은 그 원 아래 알약 하나다.

export const volumeItem = (mount: string) => `volume:${mount}`;
export const adapterItem = (name: string) => `adapter:${name}`;

/** 고른 적이 없을 때(`monitorItems` 없음) 보이는 항목. 데이터 폴더가 있는 디스크가 기본 디스크다 */
export function defaultItems(snap: MetricsSnapshot | null | undefined): string[] {
    const items = ['cpu', 'cpu.clock', 'cpu.temp', 'memory', 'memory.clock', 'gpu', 'gpu.clock', 'gpu.temp', 'disk_io'];
    const dataDisk = snap?.disks?.[0]?.mount;
    if (dataDisk) items.push(volumeItem(dataDisk));
    items.push('volume.kind', 'network');
    return items;
}

/** 항목 고르기 창의 한 줄 */
export interface PickerRow {
    id: string;
    labelKey?: string;
    labelVars?: Record<string, unknown>;
    label?: string;
    /** 이 PC에서 못 재는 값이면 "모름" 안내 */
    noteKey?: string;
    /** 이 줄이 붙는 원 항목 (그 원을 끄면 같이 꺼 둔다) */
    parent?: string;
}

export interface PickerGroup {
    key: string;
    titleKey: string;
    rows: PickerRow[];
    /** 고를 것이 없을 때 안내 */
    emptyKey?: string;
}

const volumeLabel = (mount: string) => mount.replace(/[\\/]+$/, '') || mount;

/** 표본에 있는 하드웨어로 항목 고르기 창을 만든다 */
export function pickerGroups(snap: MetricsSnapshot | null | undefined): PickerGroup[] {
    const gpus = snap?.gpus || [];
    const dataMounts = new Set((snap?.disks || []).map((d) => d.mount));
    const unknownIf = (missing: boolean) => (missing ? 'monitor.unknown_here' : undefined);
    return [
        {
            key: 'cpu',
            titleKey: 'monitor.cpu',
            rows: [
                { id: 'cpu', labelKey: 'monitor.pick_usage' },
                { id: 'cpu.clock', labelKey: 'monitor.pick_clock', parent: 'cpu', noteKey: unknownIf(snap?.cpu?.clock_mhz == null) },
                { id: 'cpu.temp', labelKey: 'monitor.pick_temp', parent: 'cpu', noteKey: unknownIf(snap?.cpu?.temp_c == null) },
            ],
        },
        {
            key: 'memory',
            titleKey: 'monitor.memory',
            rows: [
                { id: 'memory', labelKey: 'monitor.pick_usage' },
                {
                    id: 'memory.clock',
                    labelKey: 'monitor.pick_clock',
                    parent: 'memory',
                    noteKey: unknownIf(snap?.memory?.clock_mhz == null),
                },
            ],
        },
        {
            key: 'gpu',
            titleKey: 'monitor.gpu_group',
            emptyKey: 'monitor.no_gpu',
            rows: gpus.length
                ? [
                      { id: 'gpu', labelKey: 'monitor.pick_usage' },
                      { id: 'gpu.clock', labelKey: 'monitor.pick_clock', parent: 'gpu', noteKey: unknownIf(gpus.every((g) => g.clock_mhz == null)) },
                      { id: 'gpu.temp', labelKey: 'monitor.pick_temp', parent: 'gpu', noteKey: unknownIf(gpus.every((g) => g.temp_c == null)) },
                  ]
                : [],
        },
        {
            key: 'disk',
            titleKey: 'monitor.disk_group',
            rows: [
                ...(snap?.volumes || []).map((v) => ({
                    id: volumeItem(v.mount),
                    labelKey: 'monitor.pick_volume',
                    labelVars: { mount: volumeLabel(v.mount) },
                    noteKey: dataMounts.has(v.mount) ? 'monitor.data_disk' : undefined,
                })),
                { id: 'volume.kind', labelKey: 'monitor.pick_kind' },
                { id: 'disk_io', labelKey: 'monitor.disk_io' },
            ],
        },
        {
            key: 'net',
            titleKey: 'monitor.network',
            rows: [
                { id: 'network', labelKey: 'monitor.pick_network_all' },
                ...(snap?.adapters || []).map((a) => ({ id: adapterItem(a.name), label: a.name })),
            ],
        },
    ];
}

/** 항목 하나를 켜고 끈 결과 목록 */
export function toggleItem(items: string[], id: string, on: boolean): string[] {
    const set = new Set(items);
    if (on) set.add(id);
    else set.delete(id);
    return [...set];
}

/**
 * 고른 항목의 원형 그래프. 각 조각의 value 합은 total 이하이고, 남는 몫이 유휴다.
 * 원 순서는 고정이다: CPU - 메모리 - GPU - 디스크 IO - 디스크들 - 네트워크 - 네트워크 카드들
 */
export function buildRings(
    snap: MetricsSnapshot | null | undefined,
    slots: Record<string, number | null>,
    peaks: Peaks = { diskIo: 0, network: 0 },
    items: string[] = defaultItems(snap),
): Ring[] {
    if (!snap) return [];
    const on = new Set(items);
    const names = Object.fromEntries((snap.instances || []).map((i) => [i.id, i.name]));
    const insts = snap.instances || [];
    const others = (value: number | undefined) => ({ key: '__others', labelKey: 'monitor.others', value: Math.max(0, value as number), color: OTHERS_COLOR });
    const idle = (value: number, labelKey = 'monitor.idle') => ({ key: '__idle', labelKey, value: Math.max(0, value), color: IDLE_COLOR });
    const all = (value: number, labelKey: string) => ({ key: '__all', labelKey, value: Math.max(0, value), color: ALL_COLOR });
    const rings: Ring[] = [];

    if (on.has('cpu')) {
        const cpu = snap.cpu || {};
        const pills: RingPill[] = [{ key: 'monitor.cores', vars: { count: cpu.cores } }];
        if (on.has('cpu.clock')) pills.push(cpu.clock_mhz ? { text: formatGhz(cpu.clock_mhz) } : { key: 'monitor.clock_unknown', unknown: true });
        if (on.has('cpu.temp')) pills.push(cpu.temp_c != null ? { text: `${Math.round(cpu.temp_c)} °C` } : { key: 'monitor.temp_unknown', unknown: true });
        rings.push({
            key: 'cpu',
            titleKey: 'monitor.cpu',
            value: percentValue(cpu.total_percent),
            unit: '%',
            tone: 'cpu',
            pills,
            total: 100,
            format: (v: number) => `${v.toFixed(1)}%`,
            segments: [
                ...instanceSegments(insts.map((i) => ({ id: i.id, value: i.cpu_percent })), slots, names),
                others(cpu.others_percent),
                idle(100 - (cpu.total_percent || 0)),
            ],
        });
    }

    if (on.has('memory')) {
        const mem = snap.memory || {};
        const pills: RingPill[] = [{ text: `${formatBytes(mem.used_bytes as number)} / ${formatBytes(mem.total_bytes as number)}` }];
        if (on.has('memory.clock')) pills.push(mem.clock_mhz ? { text: `${mem.clock_mhz} MT/s` } : { key: 'monitor.clock_unknown', unknown: true });
        rings.push({
            key: 'memory',
            titleKey: 'monitor.memory',
            value: percentValue(mem.total_bytes ? ((mem.used_bytes as number) / mem.total_bytes) * 100 : NaN),
            unit: '%',
            tone: 'memory',
            pills,
            total: mem.total_bytes || 0,
            format: formatBytes,
            segments: [
                ...instanceSegments(insts.map((i) => ({ id: i.id, value: i.memory_bytes })), slots, names),
                others(mem.others_bytes),
                idle((mem.total_bytes || 0) - (mem.used_bytes || 0), 'monitor.free'),
            ],
        });
    }

    if (on.has('gpu')) {
        const gpus = snap.gpus || [];
        gpus.forEach((g, i) => {
            const pills: RingPill[] = [];
            if (on.has('gpu.clock')) pills.push(g.clock_mhz ? { text: formatGhz(g.clock_mhz) } : { key: 'monitor.clock_unknown', unknown: true });
            if (on.has('gpu.temp')) pills.push(g.temp_c != null ? { text: `${Math.round(g.temp_c)} °C` } : { key: 'monitor.temp_unknown', unknown: true });
            const util = g.util_percent ?? null;
            rings.push({
                key: gpus.length > 1 ? `gpu:${i}` : 'gpu',
                titleKey: g.name ? 'monitor.gpu_named' : 'monitor.gpu',
                titleVars: { name: g.name },
                value: percentValue(util),
                unit: util == null ? '' : '%',
                tone: 'gpu',
                pills,
                total: 100,
                format: (v: number) => `${v.toFixed(1)}%`,
                noteKey: util == null ? 'monitor.gpu_util_unknown' : null,
                segments: util == null ? [] : [all(util, 'monitor.gpu_usage'), idle(100 - util)],
            });
        });
    }

    if (on.has('disk_io')) {
        const io = snap.disk_io || {};
        const ioTotal = (io.read_bps || 0) + (io.write_bps || 0);
        const ioScale = Math.max(peaks.diskIo, ioTotal, MIN_RATE_SCALE);
        const [value, unit] = splitBytes(ioTotal);
        rings.push({
            key: 'disk_io',
            titleKey: 'monitor.disk_io',
            value,
            unit: `${unit}/s`,
            tone: 'disk',
            pills: [
                { key: 'monitor.read', vars: { value: formatRate(io.read_bps || 0) } },
                { key: 'monitor.write', vars: { value: formatRate(io.write_bps || 0) } },
            ],
            total: ioScale,
            format: formatRate,
            // 실제 디스크 IO는 인스턴스별로 나누지 않는다: 프로세스 IO 카운터는 캐시 읽기까지 세서 몫이 맞지 않는다
            noteKey: io.source === 'process' ? 'monitor.disk_io_process_note' : 'monitor.disk_io_note',
            segments: [all(ioTotal, 'monitor.all_disk_io'), idle(ioScale - ioTotal)],
        });
    }

    // 디스크: 데몬이 준 모든 디스크(volumes) 중 고른 것. 데이터·인스턴스 폴더가 있는 디스크면 인스턴스 몫을 나눈다
    const volumes = snap.volumes?.length
        ? snap.volumes
        : (snap.disks || []).map((d) => ({ mount: d.mount, total_bytes: d.total_bytes, available_bytes: d.available_bytes, kind: undefined, rpm: undefined }));
    for (const vol of volumes) {
        if (!on.has(volumeItem(vol.mount))) continue;
        const used = vol.total_bytes - vol.available_bytes;
        const shares = (snap.disks || []).find((d) => d.mount === vol.mount);
        const pills: RingPill[] = [{ key: 'monitor.free_pill', vars: { free: formatBytes(vol.available_bytes) } }];
        if (on.has('volume.kind')) pills.push(kindPill(vol.kind, vol.rpm));
        rings.push({
            key: volumeItem(vol.mount),
            titleKey: 'monitor.volume',
            titleVars: { mount: volumeLabel(vol.mount) },
            value: percentValue(vol.total_bytes ? (used / vol.total_bytes) * 100 : NaN),
            unit: '%',
            tone: 'disk',
            pills,
            total: vol.total_bytes,
            format: formatBytes,
            noteKey: shares && !shares.sizes_at_ms ? 'monitor.sizes_pending' : null,
            segments: shares
                ? [
                      ...instanceSegments(shares.instances.map((s) => ({ id: s.id, value: s.bytes })), slots, names),
                      others(shares.others_bytes),
                      idle(vol.available_bytes, 'monitor.free'),
                  ]
                : [all(used, 'monitor.used'), idle(vol.available_bytes, 'monitor.free')],
        });
    }

    const rateRing = (key: string, titleKey: string, titleVars: Record<string, unknown> | undefined, rx: number, tx: number, peak: number, noteKey: string | null): Ring => {
        const total = rx + tx;
        const scale = Math.max(peak, total, MIN_RATE_SCALE);
        const [value, unit] = splitBytes(total);
        return {
            key,
            titleKey,
            titleVars,
            value,
            unit: `${unit}/s`,
            tone: 'net',
            pills: [
                { key: 'monitor.rx', vars: { value: formatRate(rx) } },
                { key: 'monitor.tx', vars: { value: formatRate(tx) } },
            ],
            total: scale,
            format: formatRate,
            noteKey,
            segments: [all(total, 'monitor.all_traffic'), idle(scale - total)],
        };
    };

    if (on.has('network')) {
        const net = snap.network || {};
        const trace = snap.net_trace;
        const status = trace?.status || 'off';
        const ring = rateRing(
            'network',
            'monitor.network',
            undefined,
            net.rx_bps || 0,
            net.tx_bps || 0,
            peaks.network,
            NET_TRACE_NOTES.has(status) ? `monitor.net_trace_${status}` : 'monitor.network_note',
        );
        // 고급 옵션으로 서버별로 셀 때: 인스턴스 - 나머지 프로세스 - 유휴. 아니면 전체 트래픽 한 조각
        if (trace && status === 'on') {
            const sum = (rx?: number | null, tx?: number | null) => (rx || 0) + (tx || 0);
            const segs = [
                ...instanceSegments(insts.map((i) => ({ id: i.id, value: sum(i.net_rx_bps, i.net_tx_bps) })), slots, names),
                others(sum(trace.others_rx_bps, trace.others_tx_bps)),
            ];
            const used = segs.reduce((acc, seg) => acc + seg.value, 0);
            ring.total = Math.max(ring.total, used);
            ring.segments = [...segs, idle(ring.total - used)];
            ring.noteKey = 'monitor.scale_note';
        }
        rings.push(ring);
    }
    for (const a of snap.adapters || []) {
        if (!on.has(adapterItem(a.name))) continue;
        rings.push(rateRing(adapterItem(a.name), 'monitor.adapter', { name: a.name }, a.rx_bps || 0, a.tx_bps || 0, peaks.adapters?.[a.name] || 0, 'monitor.scale_note'));
    }
    return rings;
}

function kindPill(kind: string | undefined, rpm: number | null | undefined): RingPill {
    if (kind === 'ssd') return { text: 'SSD' };
    if (kind === 'hdd') return rpm ? { text: `HDD ${rpm}rpm` } : { text: 'HDD' };
    return { key: 'monitor.kind_unknown', unknown: true };
}

/** 포트 표 한 줄 */
export interface PortRow extends PortTraffic {
    name: string;
    color: string;
}

/** 포트별 트래픽을 서버 목록 순서, 같은 서버 안에서는 포트 번호 순으로. 세지 않을 때는 빈 목록 */
export function portRows(snap: MetricsSnapshot | null | undefined, slots: Record<string, number | null>): PortRow[] {
    if (snap?.net_trace?.status !== 'on') return [];
    const names = Object.fromEntries((snap.instances || []).map((i) => [i.id, i.name]));
    const order = (id: string) => slots[id] ?? Number.MAX_SAFE_INTEGER;
    return (snap.net_trace.ports || [])
        .map((p) => ({ ...p, name: names[p.instance_id] || p.instance_id, color: seriesColor(slots[p.instance_id]) }))
        .sort((a, b) => order(a.instance_id) - order(b.instance_id) || a.instance_id.localeCompare(b.instance_id) || a.port - b.port);
}
