import type { PointerEvent as ReactPointerEvent, MouseEvent as ReactMouseEvent } from 'react';
import { useEffect, useRef, useState } from 'react';

const INTERACTIVE = 'input, textarea, select, button, a';

/**
 * 마우스·손가락으로 끌어 놓기 (pointer 이벤트). HTML 끌어 놓기(draggable)는 쓰지 않는다:
 * Windows 웹뷰(WebView2)는 창의 파일 끌어놓기 설정에 따라 HTML 끌기를 가로채거나 막을 수 있다.
 * 6px 넘게 움직여야 끌기로 보고, 놓은 자리에서 가장 가까운 `data-drop` 칸의 값을 `onDrop(key, value)`에 준다.
 * 끈 뒤 같은 요소에 오는 click 하나는 삼킨다. 화면이 닫히면 끌던 것을 버린다
 *
 * `bind(key, enabled)`를 끌 요소에 펼친다. `dragging`은 끄는 key, `over`는 지금 위에 있는 칸 값(문자열)
 */
export function usePointerDrag(onDrop: (key: string, value: string) => void) {
    const [drag, setDrag] = useState<{ key: string; over: string | null } | null>(null); // { key, over }
    const latest = useRef(onDrop);
    latest.current = onDrop;
    const dragged = useRef(false);
    const stop = useRef<(() => void) | null>(null);
    useEffect(() => () => stop.current?.(), []);
    const dropAt = (x: number, y: number) =>
        document.elementFromPoint?.(x, y)?.closest?.<HTMLElement>('[data-drop]') || null;

    const start = (e: ReactPointerEvent<HTMLElement>, key: string) => {
        if (e.button !== undefined && e.button > 0) return;
        // 블록 안의 입력칸·버튼을 누른 것은 끌기가 아니다 (끌 요소 자신이 버튼이면 끈다)
        const inner = (e.target as Element | null)?.closest?.(INTERACTIVE);
        if (inner && inner !== e.currentTarget) return;
        stop.current?.();
        const el = e.currentTarget;
        const x0 = e.clientX;
        const y0 = e.clientY;
        let moving = false;
        dragged.current = false;
        const move = (ev: PointerEvent) => {
            const dx = ev.clientX - x0;
            const dy = ev.clientY - y0;
            if (!moving && Math.hypot(dx, dy) < 6) return;
            moving = true;
            el.style.transform = `translate(${dx}px, ${dy}px)`;
            // 손 아래 칸을 찾을 수 있게 끄는 요소는 잠깐 투명하게 둔다
            el.style.pointerEvents = 'none';
            const over = dropAt(ev.clientX, ev.clientY)?.dataset.drop ?? null;
            setDrag((d) => (d?.key === key && d.over === over ? d : { key, over }));
        };
        const detach = () => {
            window.removeEventListener('pointermove', move);
            window.removeEventListener('pointerup', end);
            window.removeEventListener('pointercancel', end);
            el.style.transform = '';
            el.style.pointerEvents = '';
            stop.current = null;
        };
        const end = (ev: PointerEvent) => {
            // 끄는 요소가 아직 투명할 때 손 아래 칸을 찾는다 (되돌린 뒤 찾으면 제자리 칸이 잡힌다)
            const target = moving && ev.type === 'pointerup' ? dropAt(ev.clientX, ev.clientY) : null;
            detach();
            setDrag(null);
            if (!moving) return;
            dragged.current = true;
            setTimeout(() => {
                dragged.current = false;
            }, 0);
            if (target) latest.current(key, target.dataset.drop as string);
        };
        stop.current = () => {
            detach();
            setDrag(null);
        };
        window.addEventListener('pointermove', move);
        window.addEventListener('pointerup', end);
        window.addEventListener('pointercancel', end);
    };

    const bind = (key: string, enabled = true) =>
        enabled
            ? {
                  'data-draggable': 'true',
                  onPointerDown: (e: ReactPointerEvent<HTMLElement>) => start(e, key),
                  onClickCapture: (e: ReactMouseEvent<HTMLElement>) => {
                      // 끈 요소 자신에게 온 click만 삼킨다 (블록 안 버튼 click은 그대로)
                      const inner = (e.target as Element | null)?.closest?.(INTERACTIVE);
                      if (!dragged.current || (inner && inner !== e.currentTarget)) return;
                      e.preventDefault();
                      e.stopPropagation();
                  },
              }
            : {};

    return { dragging: drag?.key ?? null, over: drag?.over ?? null, bind };
}
