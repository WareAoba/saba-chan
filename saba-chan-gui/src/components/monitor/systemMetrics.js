/**
 * 시스템 모니터 표시 계산. 값은 데몬 `/api/system/metrics`가 준 그대로 쓰고,
 * 여기서는 원형 그래프 조각(인스턴스별 - 나머지 프로세스 - 유휴)으로 나누기만 한다.
 */

/** 계열색 개수 (tokens.css --series-1..8). 넘치는 인스턴스는 "기타 인스턴스"로 묶는다 */
export const SERIES_COUNT = 8;
/** 디스크 IO·네트워크 눈금의 최소값: 거의 놀 때 작은 값이 꽉 찬 원으로 보이지 않게 */
const MIN_RATE_SCALE = 1024 * 1024;

export function formatBytes(n) {
    if (!Number.isFinite(n) || n < 0) return '—';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    let i = 0;
    let v = n;
    while (v >= 1024 && i < units.length - 1) {
        v /= 1024;
        i += 1;
    }
    return `${i === 0 ? Math.round(v) : v.toFixed(v < 10 ? 1 : 0)} ${units[i]}`;
}

export const formatRate = (bps) => `${formatBytes(bps)}/s`;
export const formatPercent = (p) => (Number.isFinite(p) ? `${Math.round(p)}%` : '—');

/**
 * 인스턴스 id → 계열색 번호. 서버 목록 순서를 따른다: 다른 인스턴스가 꺼져도 색이 바뀌지 않는다.
 * 목록에 없는 id는 null (기타로 묶인다)
 */
export function colorSlots(orderedIds) {
    const slots = {};
    orderedIds.forEach((id, i) => {
        slots[id] = i < SERIES_COUNT ? i + 1 : null;
    });
    return slots;
}

const seriesColor = (slot) => (slot ? `var(--series-${slot})` : 'var(--text-2)');
export const OTHERS_COLOR = 'var(--text-3)';
export const IDLE_COLOR = 'var(--chart-idle)';

/** 인스턴스 값들을 색 조각으로. 계열색 밖 인스턴스는 한 조각("기타 인스턴스")으로 합친다 */
function instanceSegments(values, slots, names) {
    const segs = [];
    let overflow = 0;
    for (const { id, value } of values) {
        if (!(value > 0)) continue;
        const slot = slots[id];
        if (slot) segs.push({ key: id, label: names[id] || id, value, color: seriesColor(slot) });
        else overflow += value;
    }
    segs.sort((a, b) => slots[a.key] - slots[b.key]);
    if (overflow > 0) segs.push({ key: '__more', labelKey: 'monitor.more_instances', value: overflow, color: seriesColor(null) });
    return segs;
}

/** 디스크 IO·네트워크 최대값을 새 표본으로 갱신한다 (앱을 켠 뒤 본 최고값이 눈금이 된다) */
export function nextPeaks(prev, snap) {
    return {
        diskIo: Math.max(prev.diskIo, (snap?.disk_io?.read_bps || 0) + (snap?.disk_io?.write_bps || 0)),
        network: Math.max(prev.network, (snap?.network?.rx_bps || 0) + (snap?.network?.tx_bps || 0)),
    };
}

/** 인스턴스로 나누지 않는 전체 몫의 색: 인스턴스 계열색과 헷갈리지 않는 중립색 */
export const ALL_COLOR = 'var(--text-2)';

/**
 * 원형 그래프 다섯 개. 각 조각의 value 합은 total 이하이고, 남는 몫이 유휴다.
 * @returns {{key, titleKey, center, sub, total, segments, noteKey?}[]}
 */
export function buildRings(snap, slots, peaks = { diskIo: 0, network: 0 }) {
    if (!snap) return [];
    const names = Object.fromEntries((snap.instances || []).map((i) => [i.id, i.name]));
    const insts = snap.instances || [];
    const others = (value) => ({ key: '__others', labelKey: 'monitor.others', value: Math.max(0, value), color: OTHERS_COLOR });
    const idle = (value, labelKey = 'monitor.idle') => ({ key: '__idle', labelKey, value: Math.max(0, value), color: IDLE_COLOR });
    const rings = [];

    const cpu = snap.cpu || {};
    rings.push({
        key: 'cpu',
        titleKey: 'monitor.cpu',
        center: formatPercent(cpu.total_percent),
        sub: { key: 'monitor.cores', vars: { count: cpu.cores } },
        total: 100,
        format: (v) => `${v.toFixed(1)}%`,
        segments: [
            ...instanceSegments(insts.map((i) => ({ id: i.id, value: i.cpu_percent })), slots, names),
            others(cpu.others_percent),
            idle(100 - (cpu.total_percent || 0)),
        ],
    });

    const mem = snap.memory || {};
    rings.push({
        key: 'memory',
        titleKey: 'monitor.memory',
        center: formatPercent(mem.total_bytes ? (mem.used_bytes / mem.total_bytes) * 100 : NaN),
        sub: { text: `${formatBytes(mem.used_bytes)} / ${formatBytes(mem.total_bytes)}` },
        total: mem.total_bytes || 0,
        format: formatBytes,
        segments: [
            ...instanceSegments(insts.map((i) => ({ id: i.id, value: i.memory_bytes })), slots, names),
            others(mem.others_bytes),
            idle((mem.total_bytes || 0) - (mem.used_bytes || 0), 'monitor.free'),
        ],
    });

    const io = snap.disk_io || {};
    const ioTotal = (io.read_bps || 0) + (io.write_bps || 0);
    const ioScale = Math.max(peaks.diskIo, ioTotal, MIN_RATE_SCALE);
    rings.push({
        key: 'disk_io',
        titleKey: 'monitor.disk_io',
        center: formatRate(ioTotal),
        sub: { key: 'monitor.read_write', vars: { read: formatRate(io.read_bps || 0), write: formatRate(io.write_bps || 0) } },
        total: ioScale,
        format: formatRate,
        // 실제 디스크 IO는 인스턴스별로 나누지 않는다: 프로세스 IO 카운터는 캐시 읽기까지 세서 몫이 맞지 않는다
        noteKey: io.source === 'process' ? 'monitor.disk_io_process_note' : 'monitor.disk_io_note',
        segments: [
            { key: '__all', labelKey: 'monitor.all_disk_io', value: ioTotal, color: ALL_COLOR },
            idle(ioScale - ioTotal),
        ],
    });

    // 데이터 폴더가 있는 디스크가 첫 번째로 온다
    const disk = (snap.disks || [])[0];
    if (disk) {
        const used = disk.total_bytes - disk.available_bytes;
        rings.push({
            key: 'disk',
            titleKey: 'monitor.disk',
            center: formatPercent(disk.total_bytes ? (used / disk.total_bytes) * 100 : NaN),
            sub: { key: 'monitor.disk_free', vars: { mount: disk.mount, free: formatBytes(disk.available_bytes) } },
            total: disk.total_bytes,
            format: formatBytes,
            noteKey: disk.sizes_at_ms ? null : 'monitor.sizes_pending',
            segments: [
                ...instanceSegments(disk.instances.map((s) => ({ id: s.id, value: s.bytes })), slots, names),
                others(disk.others_bytes),
                idle(disk.available_bytes, 'monitor.free'),
            ],
        });
    }

    const net = snap.network || {};
    const netTotal = (net.rx_bps || 0) + (net.tx_bps || 0);
    const netScale = Math.max(peaks.network, netTotal, MIN_RATE_SCALE);
    rings.push({
        key: 'network',
        titleKey: 'monitor.network',
        center: formatRate(netTotal),
        sub: { key: 'monitor.rx_tx', vars: { rx: formatRate(net.rx_bps || 0), tx: formatRate(net.tx_bps || 0) } },
        total: netScale,
        format: formatRate,
        // 프로세스별 네트워크는 데몬이 아직 못 잰다: 전체 트래픽 한 조각만
        noteKey: net.per_instance ? 'monitor.scale_note' : 'monitor.network_note',
        segments: [{ key: '__all', labelKey: 'monitor.all_traffic', value: netTotal, color: ALL_COLOR }, idle(netScale - netTotal)],
    });
    return rings;
}
