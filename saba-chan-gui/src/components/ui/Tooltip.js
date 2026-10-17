/**
 * 🐟 사바쨩 — Custom Tooltip System
 *
 * data-tooltip 속성을 가진 모든 요소에 대해  말풍선 스타일의 인앱 커스텀 tooltip을 표시한다.
 * React Portal을 사용하여 document.body에 렌더링하므로
 * 부모의 overflow:hidden 등에 영향받지 않는다.
 *
 * 사용법:
 *   <TooltipProvider>
 *     <App />
 *   </TooltipProvider>
 *
 *   <button data-tooltip="도움말 텍스트">…</button>
 */

import { useState, useEffect, useCallback, useRef } from 'react';
import { createPortal } from 'react-dom';

const SHOW_DELAY = 350;   // ms before showing
const HIDE_DELAY = 100;   // ms before hiding
const GAP = 8;            // px gap between element and tooltip
const EDGE_MARGIN = 8;    // px minimum distance from viewport edge

/**
 * 화면 경계를 고려하여 tooltip 위치와 배치(top/bottom)를 계산한다.
 *
 * @param {DOMRect} rect     - 대상 요소의 getBoundingClientRect()
 * @param {number}  tipW     - tooltip 너비
 * @param {number}  tipH     - tooltip 높이
 * @param {number}  vpW      - viewport 너비
 * @param {number}  vpH      - viewport 높이
 * @returns {{ top: number, left: number, placement: 'top'|'bottom', arrowLeft: number }}
 */
export function computeTooltipPosition(rect, tipW, tipH, vpW, _vpH) {
    const elemCenterX = rect.left + rect.width / 2;
    let placement = 'top';
    let top;

    // 위쪽 공간이 부족하면 아래에 표시
    if (rect.top - tipH - GAP < EDGE_MARGIN) {
        placement = 'bottom';
        top = rect.bottom + GAP;
    } else {
        top = rect.top - tipH - GAP;
    }

    // 수평 중앙 정렬 후 경계 클램프
    let left = elemCenterX - tipW / 2;
    left = Math.max(EDGE_MARGIN, Math.min(left, vpW - tipW - EDGE_MARGIN));

    // arrow의 수평 위치: 요소 중앙에 맞춤 (tooltip의 left 기준 상대값)
    let arrowLeft = elemCenterX - left;
    arrowLeft = Math.max(12, Math.min(arrowLeft, tipW - 12));

    return { top, left, placement, arrowLeft };
}

export function TooltipProvider({ children }) {
    const [tooltip, setTooltip] = useState(null); // { text, top, left, placement, arrowLeft }
    const showTimer = useRef(null);
    const hideTimer = useRef(null);
    const currentTarget = useRef(null);
    const tooltipRef = useRef(null);

    const clearTimers = useCallback(() => {
        if (showTimer.current) { clearTimeout(showTimer.current); showTimer.current = null; }
        if (hideTimer.current) { clearTimeout(hideTimer.current); hideTimer.current = null; }
    }, []);

    const showTooltip = useCallback((target, text) => {
        clearTimers();
        currentTarget.current = target;

        showTimer.current = setTimeout(() => {
            // 1차 렌더: 일단 화면 밖에 놓고 크기를 측정
            setTooltip({ text, top: -9999, left: -9999, placement: 'top', arrowLeft: 0, measuring: true });
        }, SHOW_DELAY);
    }, [clearTimers]);

    // 측정 후 위치 보정
    useEffect(() => {
        if (!tooltip?.measuring || !tooltipRef.current || !currentTarget.current) return;
        const tipEl = tooltipRef.current;
        const tipW = tipEl.offsetWidth;
        const tipH = tipEl.offsetHeight;
        const vpW = window.innerWidth;
        const vpH = window.innerHeight;
        const rect = currentTarget.current.getBoundingClientRect();
        const pos = computeTooltipPosition(rect, tipW, tipH, vpW, vpH);
        setTooltip((prev) => ({ ...prev, ...pos, measuring: false }));
    }, [tooltip?.measuring]);

    const hide = useCallback(() => {
        clearTimers();
        hideTimer.current = setTimeout(() => {
            setTooltip(null);
            currentTarget.current = null;
        }, HIDE_DELAY);
    }, [clearTimers]);

    useEffect(() => {
        const handleMouseEnter = (e) => {
            const target = e.target.closest?.('[data-tooltip]');
            if (!target) return;
            const text = target.getAttribute('data-tooltip');
            if (!text) return;
            showTooltip(target, text);
        };

        const handleMouseLeave = (e) => {
            const target = e.target.closest?.('[data-tooltip]');
            if (!target) return;
            hide();
        };

        // scroll/resize 시 즉시 숨기기
        const handleDismiss = () => {
            clearTimers();
            setTooltip(null);
            currentTarget.current = null;
        };

        document.addEventListener('mouseenter', handleMouseEnter, true);
        document.addEventListener('mouseleave', handleMouseLeave, true);
        window.addEventListener('scroll', handleDismiss, true);
        window.addEventListener('resize', handleDismiss);

        return () => {
            clearTimers();
            document.removeEventListener('mouseenter', handleMouseEnter, true);
            document.removeEventListener('mouseleave', handleMouseLeave, true);
            window.removeEventListener('scroll', handleDismiss, true);
            window.removeEventListener('resize', handleDismiss);
        };
    }, [showTooltip, hide, clearTimers]);

    return (
        <>
            {children}
            {tooltip && createPortal(
                <div
                    ref={tooltipRef}
                    className={`saba-tooltip saba-tooltip-${tooltip.placement}`}
                    style={{
                        top: tooltip.top,
                        left: tooltip.left,
                        opacity: tooltip.measuring ? 0 : 1,
                    }}
                    role="tooltip"
                >
                    {tooltip.text}
                    <span
                        className="saba-tooltip-arrow"
                        style={{ left: tooltip.arrowLeft }}
                    />
                </div>,
                document.body,
            )}
        </>
    );
}
