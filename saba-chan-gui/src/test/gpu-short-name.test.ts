import { describe, expect, it } from 'vitest';
import { shortGpuName } from '../components/monitor/systemMetrics';

// 시스템 모니터는 그래픽 카드 이름 전체를 보이지 않고 모델만 보인다 (사용자 11-07)
describe('shortGpuName', () => {
    it.each([
        ['NVIDIA GeForce RTX 5090', 'RTX 5090'],
        ['AMD Radeon RX 7900 XTX', 'RX 7900 XTX'],
        ['Intel(R) UHD Graphics 770', 'UHD 770'],
        ['AMD Radeon Graphics', 'Radeon'],
    ])('%s → %s', (full, short) => {
        expect(shortGpuName(full)).toBe(short);
    });
});
