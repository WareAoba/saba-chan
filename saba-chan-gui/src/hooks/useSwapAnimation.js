import { useEffect, useRef } from 'react';

/** 사용자가 애니메이션을 껐거나 시스템이 움직임 줄이기를 원하면 false */
export function motionAllowed() {
    if (typeof window === 'undefined' || !document.documentElement) return false;
    if (document.querySelector('[data-no-transitions="true"]')) return false;
    return !window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
}

/**
 * key가 바뀔 때마다 ref 요소를 살짝 떠오르며 나타나게 한다 (다시 마운트하지 않는다).
 * 처음 그릴 때는 건너뛴다.
 */
export function useSwapAnimation(key, { distance = 6, duration = 240 } = {}) {
    const ref = useRef(null);
    const first = useRef(true);
    // biome-ignore lint/correctness/useExhaustiveDependencies: key가 바뀔 때마다 다시 움직인다
    useEffect(() => {
        if (first.current) {
            first.current = false;
            return;
        }
        const el = ref.current;
        if (!el?.animate || !motionAllowed()) return;
        el.animate(
            [
                { opacity: 0.35, transform: `translateY(${distance}px)` },
                { opacity: 1, transform: 'none' },
            ],
            { duration, easing: 'cubic-bezier(0.2, 0.8, 0.2, 1)' },
        );
    }, [key, distance, duration]);
    return ref;
}
