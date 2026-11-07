import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { MetricRing } from '../components/monitor/MonitorRing';
import { buildRings } from '../components/monitor/systemMetrics';

// 서버로 나누지 않는 몫은 원 종류의 흐린 바탕색으로, 서버 몫만 계열색으로 덧칠한다 (사용자 11-07)
const t = ((k: string) => k) as never;
const strokes = (el: HTMLElement) => [...el.querySelectorAll('.monitor-arc')].map((a) => (a as SVGElement).style.stroke);

describe('원 바탕색과 서버 몫', () => {
    it('GPU 전체 몫은 바탕색이고 원은 자기 종류를 밝힌다', () => {
        const [ring] = buildRings({ gpus: [{ name: 'GPU', util_percent: 18 }] } as never, {}, undefined, ['gpu']);
        const { container } = render(<MetricRing ring={ring} t={t} />);
        expect(container.querySelector('.monitor-ring')?.getAttribute('data-tone')).toBe('gpu');
        expect(strokes(container)).toEqual(['var(--ring-base)']);
    });

    it('CPU는 서버 몫만 계열색이고 나머지 프로세스는 바탕색이다', () => {
        const snap = { cpu: { total_percent: 40, others_percent: 30, cores: 16 }, instances: [{ id: 'a', name: 'a', cpu_percent: 10, memory_bytes: 0 }] };
        const [ring] = buildRings(snap as never, { a: 1 }, undefined, ['cpu']);
        const { container } = render(<MetricRing ring={ring} t={t} />);
        expect(strokes(container)).toEqual(['var(--series-1)', 'var(--ring-base)']);
    });
});
