import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

const SHOW_DELAY = 350;
const GAP = 8;
const EDGE = 8;

/**
 * 말풍선 위치: 위에 자리가 없으면 아래로, 가로는 화면 안으로 민다.
 * @returns {{ top: number, left: number, placement: 'top'|'bottom', arrowLeft: number }}
 */
export function computeTooltipPosition(rect, tipW, tipH, vpW) {
    const center = rect.left + rect.width / 2;
    const placement = rect.top - tipH - GAP < EDGE ? 'bottom' : 'top';
    const top = placement === 'top' ? rect.top - tipH - GAP : rect.bottom + GAP;
    const left = Math.max(EDGE, Math.min(center - tipW / 2, vpW - tipW - EDGE));
    const arrowLeft = Math.max(12, Math.min(center - left, tipW - 12));
    return { top, left, placement, arrowLeft };
}

/**
 * `data-tooltip`이 붙은 요소 위에 말풍선을 띄운다. body에 포털로 그려서
 * 부모의 overflow:hidden에 잘리지 않는다. 마우스와 키보드 포커스 둘 다 받는다.
 * 아이콘만 있는 버튼은 title 대신 `data-tooltip` + `aria-label`을 쓴다.
 */
export function TooltipProvider({ children }) {
    const [tip, setTip] = useState(null); // { text, target, pos? }
    const timer = useRef(null);
    const tipRef = useRef(null);
    const current = useRef(null); // 띄웠거나 띄우려는 요소 (자식 사이를 움직일 때 다시 시작하지 않게)

    const hide = useCallback(() => {
        clearTimeout(timer.current);
        current.current = null;
        setTip(null);
    }, []);

    useEffect(() => {
        const targetOf = (e) => e.target?.closest?.('[data-tooltip]');
        const show = (e, delay) => {
            const target = targetOf(e);
            const text = target?.getAttribute('data-tooltip');
            if (!text || target === current.current) return;
            clearTimeout(timer.current);
            current.current = target;
            timer.current = setTimeout(() => setTip({ text, target }), delay);
        };
        const leave = (e) => {
            const target = targetOf(e);
            if (target && !target.contains(e.relatedTarget)) hide();
        };
        const over = (e) => show(e, SHOW_DELAY);
        const focus = (e) => {
            // 키보드로 왔을 때만 (클릭 포커스로 매번 뜨면 성가시다)
            if (e.target?.matches?.(':focus-visible')) show(e, 0);
        };
        document.addEventListener('mouseover', over);
        document.addEventListener('mouseout', leave);
        document.addEventListener('focusin', focus);
        document.addEventListener('focusout', leave);
        document.addEventListener('pointerdown', hide, true);
        window.addEventListener('scroll', hide, true);
        window.addEventListener('resize', hide);
        return () => {
            clearTimeout(timer.current);
            document.removeEventListener('mouseover', over);
            document.removeEventListener('mouseout', leave);
            document.removeEventListener('focusin', focus);
            document.removeEventListener('focusout', leave);
            document.removeEventListener('pointerdown', hide, true);
            window.removeEventListener('scroll', hide, true);
            window.removeEventListener('resize', hide);
        };
    }, [hide]);

    // 처음엔 안 보이게 그려 크기를 잰 뒤 자리를 잡는다
    useLayoutEffect(() => {
        if (!tip || tip.pos || !tipRef.current) return;
        if (!tip.target.isConnected) return setTip(null);
        const el = tipRef.current;
        const pos = computeTooltipPosition(
            tip.target.getBoundingClientRect(),
            el.offsetWidth,
            el.offsetHeight,
            window.innerWidth,
        );
        setTip((prev) => (prev ? { ...prev, pos } : prev));
    }, [tip]);

    return (
        <>
            {children}
            {tip &&
                createPortal(
                    <div
                        ref={tipRef}
                        role="tooltip"
                        className={`saba-tooltip saba-tooltip-${tip.pos?.placement || 'top'}`}
                        style={
                            tip.pos
                                ? { top: tip.pos.top, left: tip.pos.left }
                                : { top: -9999, left: -9999, visibility: 'hidden' }
                        }
                    >
                        {tip.text}
                        <span className="saba-tooltip-arrow" style={{ left: tip.pos?.arrowLeft ?? 0 }} />
                    </div>,
                    document.body,
                )}
        </>
    );
}
