/**
 * 커스텀 Tooltip 시스템 테스트
 *
 * 검증 항목:
 * 1. data-tooltip 속성이 있는 요소에 마우스를 올리면 커스텀 tooltip이 표시된다
 * 2. 마우스를 떼면 tooltip이 사라진다
 * 3. 빈 data-tooltip 값에서는 tooltip이 표시되지 않는다
 * 4. Portal을 통해 document.body에 렌더링된다
 * 5. 말풍선 스타일의 arrow가 존재한다
 * 6. 위치 계산 유틸 함수가 올바르게 동작한다
 */
import { describe, expect, it, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, fireEvent, act } from '@testing-library/react';
import { TooltipProvider } from '../components/ui/Tooltip';
import { computeTooltipPosition } from '../components/ui/Tooltip';

describe('TooltipProvider', () => {
    beforeEach(() => {
        vi.useFakeTimers();
    });
    afterEach(() => {
        vi.useRealTimers();
    });

    it('data-tooltip 속성이 있는 요소 hover 시 커스텀 tooltip이 표시된다', () => {
        render(
            <TooltipProvider>
                <button type="button" data-tooltip="테스트 툴팁">버튼</button>
            </TooltipProvider>,
        );

        const btn = screen.getByText('버튼');
        fireEvent.mouseEnter(btn);
        act(() => { vi.advanceTimersByTime(400); });

        const tooltip = document.querySelector('.saba-tooltip');
        expect(tooltip).toBeInTheDocument();
        expect(tooltip.textContent).toBe('테스트 툴팁');
    });

    it('마우스를 떼면 tooltip이 사라진다', () => {
        render(
            <TooltipProvider>
                <button type="button" data-tooltip="테스트 툴팁">버튼</button>
            </TooltipProvider>,
        );

        const btn = screen.getByText('버튼');
        fireEvent.mouseEnter(btn);
        act(() => { vi.advanceTimersByTime(400); });
        expect(document.querySelector('.saba-tooltip')).toBeInTheDocument();

        fireEvent.mouseLeave(btn);
        act(() => { vi.advanceTimersByTime(200); });
        expect(document.querySelector('.saba-tooltip')).not.toBeInTheDocument();
    });

    it('빈 data-tooltip 값에서는 tooltip이 표시되지 않는다', () => {
        render(
            <TooltipProvider>
                <button type="button" data-tooltip="">버튼</button>
            </TooltipProvider>,
        );

        const btn = screen.getByText('버튼');
        fireEvent.mouseEnter(btn);
        act(() => { vi.advanceTimersByTime(400); });

        expect(document.querySelector('.saba-tooltip')).not.toBeInTheDocument();
    });

    it('Portal로 document.body에 렌더링된다', () => {
        render(
            <TooltipProvider>
                <div style={{ overflow: 'hidden' }}>
                    <button type="button" data-tooltip="포탈 테스트">버튼</button>
                </div>
            </TooltipProvider>,
        );

        const btn = screen.getByText('버튼');
        fireEvent.mouseEnter(btn);
        act(() => { vi.advanceTimersByTime(400); });

        const tooltip = document.querySelector('.saba-tooltip');
        expect(tooltip).toBeInTheDocument();
        // Portal이므로 body의 직계 자식
        expect(tooltip.parentElement).toBe(document.body);
    });

    it('말풍선 arrow 요소가 존재한다', () => {
        render(
            <TooltipProvider>
                <button type="button" data-tooltip="화살표 테스트">버튼</button>
            </TooltipProvider>,
        );

        const btn = screen.getByText('버튼');
        fireEvent.mouseEnter(btn);
        act(() => { vi.advanceTimersByTime(400); });

        const arrow = document.querySelector('.saba-tooltip-arrow');
        expect(arrow).toBeInTheDocument();
    });

    it('네이티브 title 속성이 아닌 커스텀 DOM으로 렌더링된다', () => {
        render(
            <TooltipProvider>
                <button type="button" data-tooltip="커스텀 확인">버튼</button>
            </TooltipProvider>,
        );

        const btn = screen.getByText('버튼');
        // 원래 title 속성은 없어야 한다
        expect(btn).not.toHaveAttribute('title');

        fireEvent.mouseEnter(btn);
        act(() => { vi.advanceTimersByTime(400); });

        // 커스텀 DOM tooltip이 나와야 한다
        const tooltip = document.querySelector('.saba-tooltip');
        expect(tooltip).toBeInTheDocument();
        expect(tooltip.tagName).not.toBe('TITLE');
    });

    it('가장 가까운 data-tooltip 속성을 찾아서 보여준다 (자식 요소에서 hover)', () => {
        render(
            <TooltipProvider>
                <button type="button" data-tooltip="부모 툴팁">
                    <span>내부 아이콘</span>
                </button>
            </TooltipProvider>,
        );

        const inner = screen.getByText('내부 아이콘');
        fireEvent.mouseEnter(inner);
        act(() => { vi.advanceTimersByTime(400); });

        const tooltip = document.querySelector('.saba-tooltip');
        expect(tooltip).toBeInTheDocument();
        expect(tooltip.textContent).toBe('부모 툴팁');
    });
});

describe('computeTooltipPosition', () => {
    const makeRect = (top, left, width, height) => ({
        top,
        left,
        width,
        height,
        right: left + width,
        bottom: top + height,
    });

    it('기본 위치는 요소 위쪽 중앙이다', () => {
        const rect = makeRect(200, 300, 100, 40);
        const pos = computeTooltipPosition(rect, 120, 30, 1200, 800);
        // tooltip 중앙이 요소 중앙에 맞춰져야 함
        expect(pos.placement).toBe('top');
        expect(pos.left).toBeCloseTo(300 + 50 - 60, 0); // element center - tooltip half width
    });

    it('요소가 상단에 가까우면 아래쪽에 표시된다', () => {
        const rect = makeRect(10, 300, 100, 40);
        const pos = computeTooltipPosition(rect, 120, 30, 1200, 800);
        expect(pos.placement).toBe('bottom');
    });

    it('왼쪽 경계를 벗어나지 않도록 클램프된다', () => {
        const rect = makeRect(200, 5, 30, 40);
        const pos = computeTooltipPosition(rect, 200, 30, 1200, 800);
        expect(pos.left).toBeGreaterThanOrEqual(8);
    });

    it('오른쪽 경계를 벗어나지 않도록 클램프된다', () => {
        const rect = makeRect(200, 1150, 40, 40);
        const pos = computeTooltipPosition(rect, 200, 30, 1200, 800);
        expect(pos.left + 200).toBeLessThanOrEqual(1200 - 8);
    });
});
