import type { RefObject } from 'react';
import { useLayoutEffect } from 'react';

/**
 * 위에 붙는 머리(position: sticky) 밑으로 지나가는 형제 블록에 머리가 덮는 높이를 --sticky-clip으로 알린다.
 * 창 바탕이 반투명이면 머리를 칠해 가릴 수 없어서 CSS(styles/backdrop.css)가 이 값만큼 블록 위를 잘라 낸다
 */
export function useStickyClip(headRef: RefObject<HTMLElement | null>) {
    useLayoutEffect(() => {
        const head = headRef.current;
        if (!head) return;
        let scroller: HTMLElement | null = head.parentElement;
        while (scroller && !/(auto|scroll)/.test(getComputedStyle(scroller).overflowY))
            scroller = scroller.parentElement;
        const update = () => {
            const bottom = head.getBoundingClientRect().bottom;
            for (let el = head.nextElementSibling; el; el = el.nextElementSibling) {
                (el as HTMLElement).style.setProperty(
                    '--sticky-clip',
                    `${Math.round(bottom - el.getBoundingClientRect().top)}px`,
                );
            }
        };
        update();
        const target: HTMLElement | Window = scroller ?? window;
        target.addEventListener('scroll', update, { passive: true });
        // 탭을 바꾸거나 창 크기가 바뀌면 형제가 바뀌고 자리가 움직인다
        const parent = head.parentElement;
        const resize = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(update);
        if (parent) resize?.observe(parent);
        const mutation = parent ? new MutationObserver(update) : null;
        if (parent) mutation?.observe(parent, { childList: true });
        return () => {
            target.removeEventListener('scroll', update);
            resize?.disconnect();
            mutation?.disconnect();
        };
    }, [headRef]);
}
