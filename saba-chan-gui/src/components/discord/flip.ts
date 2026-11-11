import { type RefObject, useLayoutEffect, useRef } from 'react';

/**
 * FLIP 움직임: `capture()`로 `[data-flip]` 요소의 자리를 적어 두면, 다음 그리기에서 같은 값의 요소가
 * 옛 자리에서 새 자리로 미끄러진다 (권한 맞바꾸기의 칩이 서로 자리를 바꾼다). 움직임 줄이기 설정이면 건너뛴다
 */
export function useFlip<T extends HTMLElement>(ref: RefObject<T | null>, duration = 450) {
    const before = useRef<Map<string, DOMRect> | null>(null);
    useLayoutEffect(() => {
        const prev = before.current;
        const root = ref.current;
        before.current = null;
        if (!prev || !root || window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) return;
        let moved = 0;
        const els = [...root.querySelectorAll<HTMLElement>('[data-flip]')];
        // 새 자리는 움직임을 걸기 전에 모두 잰다 (먼저 건 움직임이 바깥 칸의 자리를 바꿔 놓지 않게)
        const now = new Map(els.map((el) => [el, el.getBoundingClientRect()]));
        els.forEach((el) => {
            const from = prev.get(el.dataset.flip as string);
            if (!from || typeof el.animate !== 'function') return;
            const to = now.get(el) as DOMRect;
            // 미끄러지는 칸 안의 요소는 그 칸에 대한 자리만 옮긴다 (칸과 함께 두 번 움직이지 않게)
            const host = el.parentElement?.closest<HTMLElement>('[data-flip]');
            const hostFrom = host ? prev.get(host.dataset.flip as string) : undefined;
            const hostTo = host ? now.get(host) : undefined;
            const dx = from.left - to.left - (hostFrom && hostTo ? hostFrom.left - hostTo.left : 0);
            const dy = from.top - to.top - (hostFrom && hostTo ? hostFrom.top - hostTo.top : 0);
            if (!dx && !dy) return;
            el.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: 'none' }], {
                duration,
                delay: Math.min(moved++, 12) * 20,
                easing: 'cubic-bezier(.3,.7,.2,1)',
                fill: 'backwards',
            });
        });
    });
    return () => {
        const root = ref.current;
        if (!root) return;
        before.current = new Map(
            [...root.querySelectorAll<HTMLElement>('[data-flip]')].map((el) => [
                el.dataset.flip as string,
                el.getBoundingClientRect(),
            ]),
        );
    };
}

/**
 * 칩이 다른 계단으로 옮겨지면 옛 자리에서 새 자리로 미끄러지게 한다 (FLIP, 앱·웹 명령 계단).
 * 칩마다 `data-chip` 으로 찾고, 움직임 줄이기 설정이면 건너뛴다.
 * 자리는 그릴 때마다 다시 재고(보기·스크롤·알림 줄이 바뀌어도 옛 값이 남지 않게), 움직임은 `dep`(계단 값)이 바뀔 때만 준다
 */
export function useChipFlip(ref: RefObject<HTMLElement | null>, dep: unknown) {
    const last = useRef(new Map<string, DOMRect>());
    const prevDep = useRef(dep);
    useLayoutEffect(() => {
        const root = ref.current;
        if (!root) return;
        const changed = prevDep.current !== dep;
        prevDep.current = dep;
        const reduce = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
        const next = new Map<string, DOMRect>();
        for (const el of root.querySelectorAll<HTMLElement>('[data-chip]')) {
            const box = el.getBoundingClientRect();
            next.set(el.dataset.chip as string, box);
            const prev = last.current.get(el.dataset.chip as string);
            if (!changed || reduce || !prev || !el.animate) continue;
            const dx = prev.left - box.left;
            const dy = prev.top - box.top;
            if (Math.abs(dx) < 1 && Math.abs(dy) < 1) continue;
            el.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: 'none' }], {
                duration: 260,
                easing: 'cubic-bezier(0.2, 0.8, 0.2, 1)',
            });
        }
        last.current = next;
    });
}
