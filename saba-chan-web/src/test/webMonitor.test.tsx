import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { MoreTab } from '../components/MoreTab';
import { initI18n } from '../i18n';
import { mockApi } from './mocks';

// 사용자 요청(2026-11-06): 앱 시스템 모니터를 웹 콘솔에서도 그대로 — 같은 원, 같은 항목 고르기
const GB = 1024 ** 3;
const snapshot = () => ({
    cpu: { cores: 8, total_percent: 40, others_percent: 10, clock_mhz: 4800, temp_c: null },
    memory: { total_bytes: 32 * GB, used_bytes: 16 * GB, others_bytes: 12 * GB, clock_mhz: 6000 },
    disk_io: { read_bps: 0, write_bps: 0, source: 'physical' },
    network: { rx_bps: 0, tx_bps: 0 },
    disks: [{ mount: '/', total_bytes: 100 * GB, available_bytes: 40 * GB, others_bytes: 50 * GB, instances: [], sizes_at_ms: 1 }],
    volumes: [{ mount: '/', name: 'root', kind: 'ssd', total_bytes: 100 * GB, available_bytes: 40 * GB, rpm: 0 }],
    adapters: [{ name: 'eth0', rx_bps: 0, tx_bps: 0 }],
    gpus: [{ name: 'RTX 5090', util_percent: 70, clock_mhz: 2600, temp_c: 60 }],
    instances: [{ id: 'a', name: 'Survival', running: true, cpu_percent: 30, memory_bytes: 4 * GB }],
});

beforeAll(async () => {
    await initI18n('ko');
});
afterEach(() => {
    cleanup();
    localStorage.clear();
});

const metric = (key: string) => document.querySelector(`[data-metric="${key}"]`);

describe('웹 PC 상태', () => {
    it('앱과 같은 원으로 기본 항목을 그린다', async () => {
        const api = mockApi({ systemMetrics: vi.fn().mockResolvedValue(snapshot()) });
        render(<MoreTab api={api} />);
        await waitFor(() => expect(metric('cpu')).not.toBeNull());
        expect(metric('cpu')!.textContent).toContain('4.80 GHz');
        expect(metric('cpu')!.textContent).toContain('온도 모름');
        expect(metric('gpu')!.textContent).toContain('RTX 5090');
        expect(metric('volume:/')).not.toBeNull();
        expect(metric('adapter:eth0')).toBeNull();
        expect(api.systemMetrics).toHaveBeenCalledWith({ sizes: true });
    });

    it('고른 항목은 이 브라우저에 남는다', async () => {
        const api = mockApi({ systemMetrics: vi.fn().mockResolvedValue(snapshot()) });
        render(<MoreTab api={api} />);
        await waitFor(() => expect(metric('gpu')).not.toBeNull());
        fireEvent.click(document.querySelector('.metrics-head button')!);
        fireEvent.click(document.querySelector('input[data-item="gpu"]')!);
        fireEvent.click(document.querySelector('input[data-item="adapter:eth0"]')!);
        expect(metric('gpu')).toBeNull();
        expect(metric('adapter:eth0')).not.toBeNull();
        const saved = JSON.parse(localStorage.getItem('saba-web.monitorItems')!);
        expect(saved).not.toContain('gpu');
        expect(saved).toContain('adapter:eth0');

        cleanup();
        render(<MoreTab api={api} />);
        await waitFor(() => expect(metric('adapter:eth0')).not.toBeNull());
        expect(metric('gpu')).toBeNull();
    });
});
