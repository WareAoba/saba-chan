import { fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { buildRings, colorSlots, nextPeaks } from '../components/monitor/systemMetrics';
import { SystemMonitor } from '../components/monitor/SystemMonitor';
import { useSettingsStore } from '../stores/useSettingsStore';

const GB = 1024 ** 3;
const MB = 1024 ** 2;

// 데몬 /api/system/metrics 응답 모양 그대로
const snapshot = (overrides = {}) => ({
    sampled_at_ms: 1,
    window_ms: 2000,
    cpu: { cores: 8, total_percent: 50, others_percent: 20 },
    memory: { total_bytes: 16 * GB, used_bytes: 10 * GB, others_bytes: 6 * GB },
    disk_io: { read_bps: 3 * MB, write_bps: 1 * MB, source: 'physical', per_instance: false },
    network: { rx_bps: 2 * MB, tx_bps: 0, per_instance: false },
    disks: [
        {
            mount: 'C:\\',
            total_bytes: 100 * GB,
            available_bytes: 40 * GB,
            others_bytes: 50 * GB,
            instances: [{ id: 'a', bytes: 10 * GB }],
            sizes_at_ms: 1,
        },
    ],
    instances: [
        { id: 'a', name: 'A', running: true, process_count: 2, cpu_percent: 30, memory_bytes: 4 * GB, disk_read_bps: 2 * MB, disk_write_bps: 1 * MB },
        { id: 'b', name: 'B', running: false, process_count: 0, cpu_percent: 0, memory_bytes: 0, disk_read_bps: 0, disk_write_bps: 0 },
    ],
    ...overrides,
});

const ring = (rings, key) => rings.find((r) => r.key === key);
const values = (r) => Object.fromEntries(r.segments.map((s) => [s.key, s.value]));

describe('시스템 모니터 원형 그래프 계산', () => {
    it('인스턴스 - 나머지 프로세스 - 유휴로 나누고 합이 전체와 같다', () => {
        const rings = buildRings(snapshot(), colorSlots(['a', 'b']));
        expect(rings.map((r) => r.key)).toEqual(['cpu', 'memory', 'disk_io', 'disk', 'network']);
        expect(values(ring(rings, 'cpu'))).toEqual({ a: 30, __others: 20, __idle: 50 });
        expect(values(ring(rings, 'memory'))).toEqual({ a: 4 * GB, __others: 6 * GB, __idle: 6 * GB });
        expect(values(ring(rings, 'disk'))).toEqual({ a: 10 * GB, __others: 50 * GB, __idle: 40 * GB });
        for (const r of rings) {
            const sum = r.segments.reduce((acc, s) => acc + s.value, 0);
            expect(sum).toBeCloseTo(r.total, 0);
        }
    });

    it('색은 서버 목록 순서를 따른다: 앞 인스턴스가 꺼져도 바뀌지 않는다', () => {
        const rings = buildRings(snapshot(), colorSlots(['b', 'a']));
        expect(ring(rings, 'cpu').segments[0]).toMatchObject({ key: 'a', color: 'var(--series-2)' });
    });

    it('계열색을 넘는 인스턴스는 기타 인스턴스 한 조각으로 묶는다', () => {
        const many = Array.from({ length: 10 }, (_, i) => ({ id: `s${i}`, name: `S${i}`, cpu_percent: 1, memory_bytes: 0 }));
        const rings = buildRings(snapshot({ instances: many }), colorSlots(many.map((i) => i.id)));
        const cpu = values(ring(rings, 'cpu'));
        expect(cpu.s7).toBe(1);
        expect(cpu.s8).toBeUndefined();
        expect(cpu.__more).toBe(2);
    });

    it('네트워크는 인스턴스로 나누지 않고, 비율 그래프는 앱을 켠 뒤 최고값을 눈금으로 쓴다', () => {
        expect(nextPeaks({ diskIo: 8 * MB, network: 1 * MB }, snapshot())).toEqual({ diskIo: 8 * MB, network: 2 * MB });
        const rings = buildRings(snapshot(), colorSlots(['a']), { diskIo: 8 * MB, network: 4 * MB });
        expect(values(ring(rings, 'network'))).toEqual({ __all: 2 * MB, __idle: 2 * MB });
        expect(ring(rings, 'network').noteKey).toBe('monitor.network_note');
        expect(ring(rings, 'disk_io').total).toBe(8 * MB);
        // 실제 디스크 IO는 나누지 않는다 (프로세스 IO는 캐시 읽기까지 세서 몫이 맞지 않는다)
        expect(values(ring(rings, 'disk_io'))).toEqual({ __all: 4 * MB, __idle: 4 * MB });
        expect(ring(rings, 'disk_io').noteKey).toBe('monitor.disk_io_note');
    });
});

describe('SystemMonitor', () => {
    beforeEach(() => {
        window.api.systemMetrics = vi.fn().mockResolvedValue(snapshot());
        useSettingsStore.setState({ systemMonitorOpen: true });
    });
    afterEach(() => {
        delete window.api.systemMetrics;
    });

    it('데몬 값을 받아 그래프 다섯 개와 범례를 그린다', async () => {
        render(<SystemMonitor servers={[{ id: 'a', name: 'A' }, { id: 'b', name: 'B' }]} />);
        await waitFor(() => expect(document.querySelectorAll('.monitor-ring')).toHaveLength(5));
        const legend = document.querySelector('.monitor-legend').textContent;
        expect(legend).toContain('A');
        // 꺼진 인스턴스는 범례에 없다
        expect(legend).not.toContain('B');
        expect(document.querySelector('[data-metric="cpu"]').textContent).toContain('50%');
    });

    it('접으면 요약 한 줄만 보이고 설정에 남긴다', async () => {
        render(<SystemMonitor servers={[{ id: 'a', name: 'A' }]} />);
        await waitFor(() => expect(document.querySelector('.monitor-ring')).not.toBeNull());
        fireEvent.click(document.querySelector('.monitor-toggle'));
        expect(useSettingsStore.getState().systemMonitorOpen).toBe(false);
        expect(document.querySelector('.monitor-ring')).toBeNull();
        expect(document.querySelector('.monitor-summary').textContent).toContain('50%');
        // 요약만 볼 때는 데몬이 폴더 크기를 재지 않게 한다
        await waitFor(() => expect(window.api.systemMetrics).toHaveBeenLastCalledWith({ sizes: false }));
    });

    it('데몬에 API가 없으면 아무것도 그리지 않는다', () => {
        window.api.systemMetrics = undefined;
        render(<SystemMonitor servers={[]} />);
        expect(document.querySelector('.monitor')).toBeNull();
    });
});
