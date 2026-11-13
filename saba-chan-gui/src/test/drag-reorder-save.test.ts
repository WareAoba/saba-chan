import { act, renderHook } from '@testing-library/react';
import type React from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { holdSavingOrder, useDragReorder } from '../hooks/useDragReorder';
import type { Server } from '../stores/useServerStore';

// 서버 카드를 끌어 순서를 바꾼다: 저장에 실패하면 알리고 되돌리며, 저장하는 사이 온 목록(옛 순서)으로 깜빡이지 않는다
const initial = [
    { id: 'ia', name: 'a' },
    { id: 'ib', name: 'b' },
] as Server[];

function setup() {
    let servers = initial;
    const setServers = vi.fn((update: (prev: Server[]) => Server[]) => {
        servers = update(servers);
    });
    const hook = renderHook(() => useDragReorder(servers, setServers));
    const cards = initial.map((s, i) => {
        const el = document.createElement('div');
        el.getBoundingClientRect = () => ({ left: 0, top: i * 100, width: 100, height: 80 }) as DOMRect;
        hook.result.current.cardRefs.current[s.id] = el;
        return el;
    });
    const point = (type: string, x: number, y: number) =>
        document.dispatchEvent(new MouseEvent(type, { clientX: x, clientY: y }));
    /** 첫 카드를 둘째 자리까지 끈다 (아직 놓지 않았다) */
    const dragFirstDown = () => {
        const down = { button: 0, target: cards[0], clientX: 10, clientY: 10 } as unknown as React.PointerEvent<HTMLElement>;
        act(() => hook.result.current.handleCardPointerDown(down, 0));
        act(() => void point('pointermove', 10, 140));
    };
    return { cards, point, dragFirstDown, names: () => servers.map((s) => s.name) };
}

beforeEach(() => {
    window.showToast = vi.fn();
});

describe('서버 순서 바꾸기 저장', () => {
    it('저장에 실패하면 알리고 원래 순서로 되돌린다', async () => {
        window.api = { instanceReorder: vi.fn().mockResolvedValue({ error: 'Could not save the order' }) } as never;
        const { dragFirstDown, point, names } = setup();
        dragFirstDown();
        await act(async () => void point('pointerup', 10, 140));
        expect(window.api.instanceReorder).toHaveBeenCalledWith(['ib', 'ia']);
        expect(names()).toEqual(['a', 'b']);
        expect((window.showToast as ReturnType<typeof vi.fn>).mock.calls[0].slice(0, 2)).toEqual(['Could not save the order', 'error']);
    });

    it('저장하는 사이 온 목록은 놓은 순서로 보이고, 저장이 끝나면 데몬 순서를 따른다', async () => {
        let finish: (res: object) => void = () => {};
        window.api = { instanceReorder: vi.fn(() => new Promise((r) => (finish = r))) } as never;
        const { dragFirstDown, point, names } = setup();
        dragFirstDown();
        await act(async () => void point('pointerup', 10, 140));
        expect(names()).toEqual(['b', 'a']);
        expect(holdSavingOrder(initial).map((s) => s.name)).toEqual(['b', 'a']);
        await act(async () => finish({ success: true }));
        expect(holdSavingOrder(initial).map((s) => s.name)).toEqual(['a', 'b']);
    });

    it('끌다가 취소되면(pointercancel) 카드를 제자리에 두고 순서를 바꾸지 않는다', async () => {
        window.api = { instanceReorder: vi.fn() } as never;
        const { cards, dragFirstDown, point, names } = setup();
        dragFirstDown();
        expect(cards[0].style.transform).not.toBe('');
        await act(async () => void point('pointercancel', 10, 140));
        expect(cards.map((c) => c.style.transform)).toEqual(['', '']);
        expect(window.api.instanceReorder).not.toHaveBeenCalled();
        expect(names()).toEqual(['a', 'b']);
    });
});
