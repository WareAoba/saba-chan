import { fireEvent, render, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { buildRings, colorSlots, defaultItems, type MetricsSnapshot, nextPeaks, pickerGroups, portRows } from '../components/monitor/systemMetrics';
import { FloatingMonitor, monitorPlacement, SystemMonitor, useSystemMetrics } from '../components/monitor/SystemMonitor';
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
        // 고른 적이 없으면 기본 항목: 데이터 폴더가 있는 디스크만, GPU는 직접 켜기 전까지 없다
        expect(rings.map((r) => r.key)).toEqual(['cpu', 'memory', 'disk_io', 'volume:C:\\', 'network']);
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

    // 사용자 요청(2026-11-06 #26): 알 수 없는 값은 "알 수 없음"으로 두지 않고 아예 보이지 않는다
    it('못 재는 클럭·온도·디스크 종류는 알약을 빼고, 사용률을 모르는 GPU는 원째 뺀다', () => {
        const snap = snapshot({
            cpu: { cores: 8, total_percent: 50, others_percent: 20, clock_mhz: null, temp_c: null },
            gpus: [{ name: null, util_percent: null, clock_mhz: null, temp_c: null }],
        });
        const rings = buildRings(snap, colorSlots([]), undefined, ['cpu', 'cpu.clock', 'cpu.temp', 'gpu', 'gpu.clock', 'gpu.temp']);
        expect(ring(rings, 'cpu').pills.slice(1)).toEqual([]);
        expect(rings.find((r) => r.key === 'gpu')).toBeUndefined();
        const disks = buildRings(
            snapshot({ volumes: [{ mount: 'C:\\', name: 'OS', kind: 'unknown', total_bytes: 100 * GB, available_bytes: 40 * GB, rpm: 0 }] }),
            colorSlots([]),
            undefined,
            ['volume:C:\\', 'volume.kind'],
        );
        expect(JSON.stringify(disks)).not.toContain('unknown');
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
        // GPU를 아직 묻지 않은 표본(gpus: null)도 켤 수 있게 줄을 보인다 (못 잰다는 안내 없이)
        const unasked = pickerGroups(snapshot({ gpus: null })).find((g) => g.key === 'gpu')!.rows;
        expect(unasked.map((r) => r.id)).toEqual(['gpu', 'gpu.clock', 'gpu.temp']);
        expect(unasked.every((r) => !r.noteKey)).toBe(true);
        // 기본 항목에는 GPU가 없다: 켜기 전까지 데몬이 NVIDIA 라이브러리(약 20MB)를 올리지 않게
        expect(defaultItems(snapshot()).some((i) => i.startsWith('gpu'))).toBe(false);
        expect(defaultItems(snapshot())).toContain('volume:C:\\');
        expect(defaultItems(snapshot())).not.toContain('volume:D:\\');
    });
});

// 고급 옵션 포트·서버별 네트워크가 셀 때의 데몬 값
const traced = () =>
    snapshot({
        network: { rx_bps: 3 * MB, tx_bps: 1 * MB, per_instance: false },
        net_trace: {
            status: 'on',
            window_ms: 2000,
            rx_bps: 3 * MB,
            tx_bps: 1 * MB,
            others_rx_bps: 1 * MB,
            others_tx_bps: 0,
            ports: [
                { instance_id: 'a', field: 'query_port', port: 27016, protocol: 'udp', rx_bps: 0, tx_bps: 0 },
                { instance_id: 'a', field: 'port', port: 25565, protocol: 'tcp', rx_bps: 2 * MB, tx_bps: 1 * MB },
            ],
        },
        instances: [
            { id: 'a', name: 'A', cpu_percent: 30, memory_bytes: 4 * GB, net_rx_bps: 2 * MB, net_tx_bps: 1 * MB },
            { id: 'b', name: 'B', cpu_percent: 0, memory_bytes: 0, net_rx_bps: 0, net_tx_bps: 0 },
        ],
    });

describe('포트·서버별 네트워크 (고급 옵션)', () => {
    it('셀 때는 네트워크 원을 인스턴스 - 나머지 프로세스 - 유휴로 나눈다', () => {
        const net = ring(buildRings(traced(), colorSlots(['a', 'b']), { diskIo: 0, network: 8 * MB }), 'network');
        expect(values(net)).toEqual({ a: 3 * MB, __others: 1 * MB, __idle: 4 * MB });
        expect(net.noteKey).toBe('monitor.scale_note');
    });

    it('옵션은 켰지만 권한이 없으면 전체 한 조각과 그 까닭을 보인다', () => {
        const snap = snapshot({ net_trace: { status: 'needs_grant' } });
        const net = ring(buildRings(snap, colorSlots(['a'])), 'network');
        expect(Object.keys(values(net))).toEqual(['__all', '__idle']);
        expect(net.noteKey).toBe('monitor.net_trace_needs_grant');
        expect(portRows(snap, colorSlots(['a']))).toEqual([]);
    });

    it('서버별로 셀 때는 가운데 숫자와 받기·보내기도 조각과 같은 ETW 값이다', () => {
        const snap = traced() as MetricsSnapshot;
        const loopback = { ...snap, network: { rx_bps: 0, tx_bps: 0, per_instance: false }, net_trace: { ...snap.net_trace!, rx_bps: 6 * MB, tx_bps: 2 * MB } };
        const net = ring(buildRings(loopback, colorSlots(['a', 'b']), { diskIo: 0, network: 0 }), 'network');
        expect([net.value, net.unit]).toEqual(['8.0', 'MB/s']);
        expect(net.pills.map((p) => p.vars?.value)).toEqual(['6.0 MB/s', '2.0 MB/s']);
    });

    it('서버별로 셀 때는 ETW 합도 최고값 눈금에 넣는다 (루프백이 있으면 카드 값보다 크다)', () => {
        const snap = traced() as MetricsSnapshot;
        const loopback = { ...snap, net_trace: { ...snap.net_trace!, rx_bps: 6 * MB } };
        expect(nextPeaks({ diskIo: 0, network: 0 }, loopback).network).toBe(7 * MB);
    });

    it('포트 표는 서버 목록 순서, 같은 서버 안에서는 포트 번호 순이다', () => {
        const rows = portRows(traced(), colorSlots(['a', 'b']));
        expect(rows.map((r) => [r.name, r.port, r.protocol])).toEqual([
            ['A', 25565, 'tcp'],
            ['A', 27016, 'udp'],
        ]);
        expect(rows[0].color).toBe('var(--series-1)');
    });
});

describe('SystemMonitor', () => {
    beforeEach(() => {
        window.api.systemMetrics = vi.fn().mockResolvedValue(snapshot());
        useSettingsStore.setState({ systemMonitorExpanded: true });
    });
    afterEach(() => {
        delete (window.api as unknown as Record<string, unknown>).systemMetrics;
    });

    it('데몬 값을 받아 기본 항목 그래프와 범례를 그린다', async () => {
        render(<SystemMonitor servers={[{ id: 'a', name: 'A' }, { id: 'b', name: 'B' }]} />);
        await waitFor(() => expect(document.querySelectorAll('.monitor-ring')).toHaveLength(5));
        // 기본 항목은 GPU를 묻지 않는다
        expect(document.querySelector('[data-metric="gpu"]')).toBeNull();
        expect(window.api.systemMetrics).toHaveBeenLastCalledWith({ sizes: true, gpu: false });
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
        expect(useSettingsStore.getState().systemMonitorExpanded).toBe(false);
        expect(document.querySelector('.monitor-ring')).toBeNull();
        expect(document.querySelector('.monitor-summary')!.textContent).toContain('50%');
        // 요약만 볼 때는 데몬이 폴더 크기를 재지 않게 한다
        await waitFor(() => expect(window.api.systemMetrics).toHaveBeenLastCalledWith({ sizes: false, gpu: false }));
    });

    it('GPU는 직접 켜야 데몬에 묻고, 켜면 설정에 남아 다음에도 켜져 있다', async () => {
        useSettingsStore.setState({ monitorItems: null });
        render(<SystemMonitor servers={[{ id: 'a', name: 'A' }]} />);
        await waitFor(() => expect(document.querySelector('.monitor-ring')).not.toBeNull());
        fireEvent.click(document.querySelector('.monitor-pick-toggle')!);
        const gpu = document.querySelector('input[data-item="gpu"]') as HTMLInputElement;
        expect(gpu.checked).toBe(false);
        fireEvent.click(gpu);
        expect(useSettingsStore.getState().monitorItems).toContain('gpu');
        await waitFor(() => expect(window.api.systemMetrics).toHaveBeenLastCalledWith({ sizes: true, gpu: true }));
        await waitFor(() => expect(document.querySelector('[data-metric="gpu"]')).not.toBeNull());
        useSettingsStore.setState({ monitorItems: null });
    });

    it('항목 고르기에서 끈 항목은 사라지고 설정에 남는다', async () => {
        useSettingsStore.setState({ monitorItems: ['cpu', 'gpu', 'gpu.clock', 'gpu.temp', 'volume:C:\\'] });
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

    it('다룰 데몬을 바꾸면 다른 데몬의 최고값이 눈금으로 이어지지 않는다', async () => {
        const withIo = (mb: number) => snapshot({ disk_io: { read_bps: mb * MB, write_bps: 0, source: 'physical', per_instance: false } });
        let target: string | null = 'pc-a';
        (window.api as unknown as Record<string, unknown>).daemonTarget = vi.fn(async () => ({ target }));
        window.api.systemMetrics = vi.fn(async () => withIo(target === 'pc-a' ? 500 : 1));
        const first = renderHook(() => useSystemMetrics(60000));
        await waitFor(() => expect(first.result.current.peaks.diskIo).toBe(500 * MB));
        first.unmount();
        target = 'pc-b';
        const second = renderHook(() => useSystemMetrics(60000));
        await waitFor(() => expect(second.result.current.peaks.diskIo).toBe(1 * MB));
        second.unmount();
        // 다시 이 PC로 돌아오면 그 데몬에서 본 최고값이 남아 있다
        target = 'pc-a';
        window.api.systemMetrics = vi.fn(async () => withIo(1));
        const third = renderHook(() => useSystemMetrics(60000));
        await waitFor(() => expect(third.result.current.peaks.diskIo).toBe(500 * MB));
        third.unmount();
        delete (window.api as unknown as Record<string, unknown>).daemonTarget;
    });

    it('포트·서버별 네트워크를 셀 때만 포트 표를 그린다', async () => {
        window.api.systemMetrics = vi.fn().mockResolvedValue(traced());
        render(<SystemMonitor servers={[{ id: 'a', name: 'A' }]} />);
        await waitFor(() => expect(document.querySelector('.monitor-ports')).not.toBeNull());
        const rows = document.querySelectorAll('.monitor-ports tbody tr');
        expect(rows).toHaveLength(2);
        expect(rows[0].textContent).toContain('25565');
        expect(rows[0].textContent).toContain('2.0 MB/s');
    });

    it('세지 않으면 포트 표가 없다', async () => {
        render(<SystemMonitor servers={[{ id: 'a', name: 'A' }]} />);
        await waitFor(() => expect(document.querySelector('.monitor-ring')).not.toBeNull());
        expect(document.querySelector('.monitor-ports')).toBeNull();
    });

    it('데몬에 API가 없으면 아무것도 그리지 않는다', () => {
        window.api.systemMetrics = undefined as never;
        render(<SystemMonitor servers={[]} />);
        expect(document.querySelector('.monitor')).toBeNull();
    });
});

describe('시스템 모니터 표시 방식 (사용자 11-11: 끔·고정·떠 있는 창)', () => {
    beforeEach(() => {
        window.api.systemMetrics = vi.fn().mockResolvedValue(snapshot());
        window.showToast = vi.fn();
        useSettingsStore.setState({ systemMonitorExpanded: true, systemMonitorMode: 'docked', systemMonitorFloat: null });
    });
    afterEach(() => {
        delete (window.api as unknown as Record<string, unknown>).systemMetrics;
        useSettingsStore.setState({ systemMonitorMode: 'docked', systemMonitorFloat: null });
    });

    it('서버 페이지의 자리: 끄면 어디에도 없고, 떠 있는 창이면 고정 칸을 쓰지 않는다', () => {
        expect(monitorPlacement('off', true, 2000)).toBe('none');
        expect(monitorPlacement('docked', true, 2000)).toBe('side');
        expect(monitorPlacement('docked', true, 1200)).toBe('top');
        expect(monitorPlacement('docked', false, 2000)).toBe('top');
        expect(monitorPlacement('floating', true, 2000)).toBe('float');
    });

    it('끄기 버튼은 끔으로 두고, 알림의 되돌리기로 원래 방식으로 돌아간다', async () => {
        render(<SystemMonitor servers={[{ id: 'a', name: 'A' }]} />);
        await waitFor(() => expect(document.querySelector('.monitor-ring')).not.toBeNull());
        fireEvent.click(document.querySelector('.monitor-hide')!);
        expect(useSettingsStore.getState().systemMonitorMode).toBe('off');
        const options = vi.mocked(window.showToast!).mock.calls[0][3]!;
        expect(options.action?.label).toBeTruthy();
        options.action!.onClick();
        expect(useSettingsStore.getState().systemMonitorMode).toBe('docked');
    });

    it('떠 있는 창은 손잡이가 있고, 접으면 요약을 두 줄 격자로 보인다', async () => {
        useSettingsStore.setState({ systemMonitorMode: 'floating' });
        render(<FloatingMonitor servers={[{ id: 'a', name: 'A' }]} />);
        await waitFor(() => expect(document.querySelector('.monitor-float .monitor-ring')).not.toBeNull());
        expect(document.querySelector('.monitor-float .monitor-grip')).not.toBeNull();
        fireEvent.click(document.querySelector('.monitor-float .monitor-toggle')!);
        const pill = document.querySelector('.monitor-float.monitor-summary')!;
        expect(pill.querySelector('.monitor-grip')).not.toBeNull();
        expect(pill.querySelector('.monitor-chips.grid')).not.toBeNull();
        expect(pill.querySelector('.monitor-hide')).not.toBeNull();
    });

    it('떠 있는 창을 끌어 놓으면 가까운 모서리 기준 자리를 설정에 남긴다', async () => {
        useSettingsStore.setState({ systemMonitorMode: 'floating' });
        render(<FloatingMonitor servers={[{ id: 'a', name: 'A' }]} />);
        await waitFor(() => expect(document.querySelector('.monitor-float .monitor-ring')).not.toBeNull());
        const layer = document.querySelector('.monitor-float-layer') as HTMLElement;
        const box = document.querySelector('.monitor-float') as HTMLElement;
        Object.defineProperties(layer, { clientWidth: { value: 1000 }, clientHeight: { value: 600 } });
        Object.defineProperties(box, { offsetWidth: { value: 200 }, offsetHeight: { value: 100 } });
        const head = box.querySelector('.monitor-head')!;
        // 기본 자리(오른쪽 위 = left 788, top 12)에서 왼쪽 가운데쯤으로 끌어 놓는다
        // 끌기 시작이 글자 선택을 시작하지 않아야 아래 콘솔 등이 함께 선택되지 않는다 (사용자 11-12)
        const notPrevented = fireEvent.pointerDown(head, { clientX: 800, clientY: 20, pointerId: 1, button: 0 });
        expect(notPrevented).toBe(false);
        fireEvent.pointerMove(head, { clientX: 300, clientY: 200, pointerId: 1 });
        fireEvent.pointerUp(head, { clientX: 300, clientY: 200, pointerId: 1 });
        expect(useSettingsStore.getState().systemMonitorFloat).toEqual({ corner: 'top-left', x: 288, y: 192 });
    });
});
