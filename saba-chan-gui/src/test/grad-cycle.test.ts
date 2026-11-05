/**
 * 그라데이션 버튼 호버 순환: 올리면 흐르고, 내리면 최소 한 바퀴를 채워 원래 그림에 부드럽게 선다.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { createCycleMotion, GRAD_CYCLE_SELECTOR, LAP_SEC } from '../utils/gradCycle';
import { applyCustomTheme, THEME_DEFAULTS } from '../utils/themeManager';

const FRAME = 1 / 60;

/** hoverSec 동안 올렸다가 내리고 멈출 때까지 돌린다 */
function run(hoverSec: number) {
    const m = createCycleMotion();
    const vels = [];
    let laps = 0;
    for (let t = 0; t < hoverSec; t += FRAME) {
        m.step(FRAME, true);
        vels.push(m.vel);
        laps = Math.max(laps, m.pos);
    }
    let frames = 0;
    while (m.step(FRAME, false)) {
        vels.push(m.vel);
        laps = Math.max(laps, m.pos);
        if (++frames > 60 * 30) throw new Error('멈추지 않음');
    }
    return { m, vels, laps };
}

describe('순환 움직임', () => {
    it('잠깐 스쳐도 한 바퀴를 다 돌고 원래 자리에 선다', () => {
        const { m, laps } = run(0.05);
        expect(laps).toBeGreaterThan(0.999);
        expect(laps).toBeLessThanOrEqual(1 + 1e-9);
        expect([m.pos, m.vel, m.moving]).toEqual([0, 0, false]);
    });

    it('오래 올리면 정수 바퀴에서 선다', () => {
        for (const sec of [1.3, 3, 7.7]) {
            const { m, laps } = run(sec);
            expect(Math.abs(laps - Math.round(laps))).toBeLessThan(1e-3);
            expect(laps).toBeGreaterThanOrEqual(sec / LAP_SEC - 0.5);
            expect(m.pos).toBe(0);
        }
    });

    it('속도는 튀지 않고 0에서 시작해 0으로 끝난다', () => {
        const { vels } = run(2);
        expect(vels[0]).toBeLessThan(0.01);
        expect(vels.at(-1)).toBeLessThan(0.01);
        for (let i = 1; i < vels.length; i++)
            expect(Math.abs(vels[i] - vels[i - 1])).toBeLessThan((1 / LAP_SEC) * 0.06);
    });

    it('감속 중에 다시 올리면 그 속도에서 이어서 돈다', () => {
        const m = createCycleMotion();
        for (let t = 0; t < 2; t += FRAME) m.step(FRAME, true);
        for (let t = 0; t < 0.3; t += FRAME) m.step(FRAME, false);
        const v = m.vel;
        m.step(FRAME, true);
        expect(Math.abs(m.vel - v)).toBeLessThan(0.02);
    });
});

describe('그라데이션 설정', () => {
    const root = document.documentElement;
    afterEach(() => applyCustomTheme(THEME_DEFAULTS));

    it('켜면 버튼이 순환 대상이 되고 두 색을 넘긴다', () => {
        applyCustomTheme({ ...THEME_DEFAULTS, useGradient: true });
        expect(root.dataset.brandGradient).toBe('on');
        expect(root.style.getPropertyValue('--brand-from')).not.toBe('');
        expect(root.style.getPropertyValue('--brand-to')).not.toBe('');
        const btn = document.createElement('button');
        btn.className = 'btn-primary';
        document.body.append(btn);
        expect(btn.matches(GRAD_CYCLE_SELECTOR)).toBe(true);
        btn.remove();
    });

    it('끄면 단색이라 순환하지 않는다', () => {
        applyCustomTheme({ ...THEME_DEFAULTS, useGradient: false });
        expect(root.dataset.brandGradient).toBe('off');
        const btn = document.createElement('button');
        btn.className = 'btn-primary';
        document.body.append(btn);
        expect(btn.matches(GRAD_CYCLE_SELECTOR)).toBe(false);
        btn.remove();
    });
});
