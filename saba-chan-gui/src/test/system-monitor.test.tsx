import { fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { buildRings, colorSlots, defaultItems, nextPeaks, pickerGroups } from '../components/monitor/systemMetrics';
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
    volumes: [
        { mount: 'C:\\', name: 'OS', kind: 'ssd', total_bytes: 100 * GB, available_bytes: 40 * GB, rpm: 0 },
        { mount: 'D:\\', name: 'Data', kind: 'hdd', total_bytes: 200 * GB, available_bytes: 150 * GB, rpm: 7200 },
    ],
    adapters: [
        { name: 'Ethernet', rx_bps: 2 * MB, tx_bps: 0 },
        { name: 'Wi-Fi', rx_bps: 0, tx_bps: 0 },
    ],
    gpus: [{ name: 'RTX 5090', util_percent: 72, clock_mhz: 2610, temp_c: 64 }],
    instances: [
        { id: 'a', name: 'A', running: true, process_count: 2, cpu_percent: 30, memory_bytes: 4 * GB, disk_read_bps: 2 * MB, disk_write_bps: 1 * MB },
        { id: 'b', name: 'B', running: false, process_count: 0, cpu_percent: 0, memory_bytes: 0, disk_read_bps: 0, disk_write_bps: 0 },
    ],
    ...overrides,
});

type Ring = ReturnType<typeof buildRings>[number];
const ring = (rings: Ring[], key: string) => rings.find((r) => r.key === key)!;
const values = (r: Ring) => Object.fromEntries(r.segments.map((s) => [s.key, s.value]));

describe('시스템 모니터 원형 그래프 계산', () => {
    it('인스턴스 - 나머지 프로세스 - 유휴로 나누고 합이 전체와 같다', () => {
        const rings = buildRings(snapshot(), colorSlots(['a', 'b']));
        // 고른 적이 없으면 기본 항목: 데이터 폴더가 있는 디스크만
        expect(rings.map((r) => r.key)).toEqual(['cpu', 'memory', 'gpu', 'disk_io', 'volume:C:\\', 'network']);
        expect(values(ring(rings, 'cpu'))).toEqual({ a: 30, __others: 20, __idle: 50 });
        expect(values(ring(rings, 'memory'))).toEqual({ a: 4 * GB, __others: 6 * GB, __idle: 6 * GB });
        expect(values(ring(rings, 'volume:C:\\'))).toEqual({ a: 10 * GB, __others: 50 * GB, __idle: 40 * GB });
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
        expect(nextPeaks({ diskIo: 8 * MB, network: 1 * MB }, snapshot())).toMatchObject({ diskIo: 8 * MB, network: 2 * MB });
        const rings = buildRings(snapshot(), colorSlots(['a']), { diskIo: 8 * MB, network: 4 * MB });
        expect(values(ring(rings, 'network'))).toEqual({ __all: 2 * MB, __idle: 2 * MB });
        expect(ring(rings, 'network').noteKey).toBe('monitor.network_note');
        expect(ring(rings, 'disk_io').total).toBe(8 * MB);
        // 실제 디스크 IO는 나누지 않는다 (프로세스 IO는 캐시 읽기까지 세서 몫이 맞지 않는다)
        expect(values(ring(rings, 'disk_io'))).toEqual({ __all: 4 * MB, __idle: 4 * MB });
        expect(ring(rings, 'disk_io').noteKey).toBe('monitor.disk_io_note');
    });
});

describe('시스템 모니터 항목 고르기', () => {
    it('고른 항목만 원으로, 고른 보조 값만 알약으로 만든다', () => {
        const items = ['cpu', 'cpu.clock', 'gpu', 'gpu.temp', 'volume:D:\\', 'volume.kind', 'adapter:Ethernet'];
        const snap = snapshot({ cpu: { cores: 8, total_percent: 50, others_percent: 20, clock_mhz: 4800, temp_c: null } });
        const rings = buildRings(snap, colorSlots(['a']), { diskIo: 0, network: 0 }, items);
        expect(rings.map((r) => r.key)).toEqual(['cpu', 'gpu', 'volume:D:\\', 'adapter:Ethernet']);
        expect(ring(rings, 'cpu').pills.map((p) => p.text ?? p.key)).toEqual(['monitor.cores', '4.80 GHz']);
        expect(ring(rings, 'gpu')).toMatchObject({ value: '72', unit: '%', titleVars: { name: 'RTX 5090' } });
        expect(ring(rings, 'gpu').pills).toEqual([{ text: '64 °C' }]);
        // 데이터 폴더가 없는 디스크는 인스턴스로 나누지 않는다
        expect(values(ring(rings, 'volume:D:\\'))).toEqual({ __all: 50 * GB, __idle: 150 * GB });
        expect(ring(rings, 'volume:D:\\').pills.at(-1)).toEqual({ text: 'HDD 7200rpm' });
        expect(ring(rings, 'adapter:Ethernet')).toMatchObject({ value: '2.0', unit: 'MB/s' });
    });

    it('못 재는 값은 모름 알약으로 둔다', () => {
        const snap = snapshot({
            cpu: { cores: 8, total_percent: 50, others_percent: 20, clock_mhz: null, temp_c: null },
            gpus: [{ name: null, util_percent: null, clock_mhz: null, temp_c: null }],
        });
        const rings = buildRings(snap, colorSlots([]), undefined, ['cpu', 'cpu.clock', 'cpu.temp', 'gpu', 'gpu.clock']);
        expect(ring(rings, 'cpu').pills.slice(1)).toEqual([
            { key: 'monitor.clock_unknown', unknown: true },
            { key: 'monitor.temp_unknown', unknown: true },
        ]);
        expect(ring(rings, 'gpu')).toMatchObject({ value: '—', titleKey: 'monitor.gpu', segments: [] });
    });

    it('네트워크 카드별 눈금은 그 카드의 최고값이다', () => {
        const peaks = nextPeaks({ diskIo: 0, network: 0, adapters: { Ethernet: 8 * MB } }, snapshot());
        expect(peaks.adapters).toEqual({ Ethernet: 8 * MB, 'Wi-Fi': 0 });
        const rings = buildRings(snapshot(), colorSlots([]), peaks, ['adapter:Ethernet']);
        expect(ring(rings, 'adapter:Ethernet').total).toBe(8 * MB);
    });

    it('고르기 창은 표본의 디스크·네트워크 카드를 줄로 보이고 데이터 디스크를 표시한다', () => {
        const groups = pickerGroups(snapshot());
        const disk = groups.find((g) => g.key === 'disk')!;
        expect(disk.rows.map((r) => r.id)).toEqual(['volume:C:\\', 'volume:D:\\', 'volume.kind', 'disk_io']);
        expect(disk.rows[0].noteKey).toBe('monitor.data_disk');
        expect(groups.find((g) => g.key === 'net')!.rows.map((r) => r.id)).toEqual(['network', 'adapter:Ethernet', 'adapter:Wi-Fi']);
        expect(groups.find((g) => g.key === 'gpu')!.rows).toHaveLength(3);
        expect(pickerGroups(snapshot({ gpus: [] })).find((g) => g.key === 'gpu')!.rows).toEqual([]);
        expect(defaultItems(snapshot())).toContain('volume:C:\\');
        expect(defaultItems(snapshot())).not.toContain('volume:D:\\');
    });
});

describe('SystemMonitor', () => {
    beforeEach(() => {
        window.api.systemMetrics = vi.fn().mockResolvedValue(snapshot());
        useSettingsStore.setState({ systemMonitorOpen: true });
    });
    afterEach(() => {
        delete (window.api as unknown as Record<string, unknown>).systemMetrics;
    });

    it('데몬 값을 받아 기본 항목 그래프와 범례를 그린다', async () => {
        render(<SystemMonitor servers={[{ id: 'a', name: 'A' }, { id: 'b', name: 'B' }]} />);
        await waitFor(() => expect(document.querySelectorAll('.monitor-ring')).toHaveLength(6));
        const legend = document.querySelector('.monitor-legend')!.textContent;
        expect(legend).toContain('A');
        // 꺼진 인스턴스는 범례에 없다
        expect(legend).not.toContain('B');
        expect(document.querySelector('[data-metric="cpu"]')!.textContent).toContain('50%');
    });

    it('접으면 요약 한 줄만 보이고 설정에 남긴다', async () => {
        render(<SystemMonitor servers={[{ id: 'a', name: 'A' }]} />);
        await waitFor(() => expect(document.querySelector('.monitor-ring')).not.toBeNull());
        fireEvent.click(document.querySelector('.monitor-toggle')!);
        expect(useSettingsStore.getState().systemMonitorOpen).toBe(false);
        expect(document.querySelector('.monitor-ring')).toBeNull();
        expect(document.querySelector('.monitor-summary')!.textContent).toContain('50%');
        // 요약만 볼 때는 데몬이 폴더 크기를 재지 않게 한다
        await waitFor(() => expect(window.api.systemMetrics).toHaveBeenLastCalledWith({ sizes: false }));
    });

    it('항목 고르기에서 끈 항목은 사라지고 설정에 남는다', async () => {
        useSettingsStore.setState({ monitorItems: null });
        render(<SystemMonitor servers={[{ id: 'a', name: 'A' }]} />);
        await waitFor(() => expect(document.querySelector('[data-metric="gpu"]')).not.toBeNull());
        fireEvent.click(document.querySelector('.monitor-pick-toggle')!);
        const gpu = document.querySelector('input[data-item="gpu"]') as HTMLInputElement;
        expect(gpu.checked).toBe(true);
        fireEvent.click(gpu);
        expect(document.querySelector('[data-metric="gpu"]')).toBeNull();
        expect(useSettingsStore.getState().monitorItems).not.toContain('gpu');
        // 원을 끄면 그 원의 보조 값은 고를 수 없다
        expect((document.querySelector('input[data-item="gpu.clock"]') as HTMLInputElement).disabled).toBe(true);
        const byItem = (id: string) => [...document.querySelectorAll<HTMLInputElement>('input[data-item]')].find((i) => i.dataset.item === id)!;
        fireEvent.click(byItem('volume:D:\\'));
        await waitFor(() => expect([...document.querySelectorAll<HTMLElement>('[data-metric]')].some((f) => f.dataset.metric === 'volume:D:\\')).toBe(true));
        useSettingsStore.setState({ monitorItems: null });
    });

    it('데몬에 API가 없으면 아무것도 그리지 않는다', () => {
        window.api.systemMetrics = undefined as never;
        render(<SystemMonitor servers={[]} />);
        expect(document.querySelector('.monitor')).toBeNull();
    });
});
