/**
 * 매트릭스 캔버스는 그리는 판(canvas.width/height)을 보이는 크기에 맞춘다. 창 크기와 상관없이 자리 크기만 바뀌면
 * (원격 데몬 전환 화면처럼 처음에 0이었다가 커지는 자리) 예전 판이 늘어나 글자가 커다랗게 깨져 보였다.
 */
import { act, render } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { MatrixRain } from '../components/shell/MatrixRain';

let observed: (() => void) | null = null;
let size = { w: 0, h: 0 };

beforeEach(() => {
    observed = null;
    size = { w: 0, h: 0 };
    vi.stubGlobal(
        'ResizeObserver',
        class {
            constructor(cb: () => void) {
                observed = cb;
            }
            observe() {}
            disconnect() {}
        },
    );
    const ctx = new Proxy({}, { get: (_t, k) => (k === 'measureText' ? () => ({ width: 10 }) : () => {}), set: () => true });
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(ctx as never);
    vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(() => size.w);
    vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockImplementation(() => size.h);
});

afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
});

it('자리 크기가 바뀌면 창 크기가 그대로여도 그리는 판을 다시 맞춘다', () => {
    const { container } = render(<MatrixRain />);
    const canvas = container.querySelector('canvas') as HTMLCanvasElement;
    size = { w: 800, h: 600 };
    act(() => observed?.());
    expect(canvas.width).toBe(Math.round(800 * Math.min(window.devicePixelRatio || 1, 2)));
    expect(canvas.height).toBe(Math.round(600 * Math.min(window.devicePixelRatio || 1, 2)));
});
