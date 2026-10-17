import { act, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { computeTooltipPosition, TooltipProvider } from '../components/ui/Tooltip';

const rect = (left, top, width = 20, height = 20) => ({ left, top, width, height, right: left + width, bottom: top + height });

describe('computeTooltipPosition', () => {
    it('자리가 있으면 위, 화면 위 끝이면 아래로 뒤집는다', () => {
        expect(computeTooltipPosition(rect(100, 200), 60, 24, 1000)).toMatchObject({ placement: 'top', top: 168 });
        expect(computeTooltipPosition(rect(100, 10), 60, 24, 1000)).toMatchObject({ placement: 'bottom', top: 38 });
    });

    it('가로는 화면 안으로 밀고, 화살표는 요소 가운데를 가리킨다', () => {
        const p = computeTooltipPosition(rect(985, 200, 10), 100, 24, 1000);
        expect(p.left).toBe(892);
        expect(p.arrowLeft).toBe(88);
        expect(computeTooltipPosition(rect(0, 200, 10), 100, 24, 1000).left).toBe(8);
    });
});

describe('TooltipProvider', () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it('잠깐 머물면 뜨고, 벗어나거나 누르면 사라진다', () => {
        render(
            <TooltipProvider>
                <button type="button" data-tooltip="새로고침" aria-label="새로고침">
                    <span className="ic">i</span>
                </button>
                <p>밖</p>
            </TooltipProvider>,
        );
        const btn = document.querySelector('button');
        fireEvent.mouseOver(btn.querySelector('.ic'));
        expect(document.querySelector('.saba-tooltip')).toBeNull();
        act(() => vi.advanceTimersByTime(400));
        const tip = document.querySelector('[role="tooltip"]');
        expect(tip.textContent).toBe('새로고침');
        expect(tip.parentElement).toBe(document.body);

        // 자식 사이 이동은 닫지 않는다
        fireEvent.mouseOut(btn.querySelector('.ic'), { relatedTarget: btn });
        expect(document.querySelector('.saba-tooltip')).not.toBeNull();

        fireEvent.mouseOut(btn, { relatedTarget: document.querySelector('p') });
        expect(document.querySelector('.saba-tooltip')).toBeNull();

        fireEvent.mouseOver(btn);
        act(() => vi.advanceTimersByTime(400));
        fireEvent.pointerDown(btn);
        expect(document.querySelector('.saba-tooltip')).toBeNull();
    });
});
