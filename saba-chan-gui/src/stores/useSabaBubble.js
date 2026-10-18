import { create } from 'zustand';

/** 말풍선이 떠 있는 최소 시간 */
export const BUBBLE_MIN_MS = 5000;
/** hold 말풍선이 풀리지 않아도 이만큼 지나면 닫는다 */
export const BUBBLE_MAX_HOLD_MS = 120000;

/**
 * 좌상단 사바쨩 아이콘에서 늘어져 나오는 말풍선 (한 번에 하나).
 * show({ kind, text, minMs?, holdKey? }): holdKey가 있으면 release(holdKey) 전까지 닫지 않는다 (최소 minMs).
 */
export const useSabaBubble = create((set, get) => ({
    bubble: null,
    show: (b) =>
        set({
            bubble: { id: `${Date.now()}-${Math.random()}`, minMs: BUBBLE_MIN_MS, released: !b.holdKey, ...b },
        }),
    release: (holdKey) => {
        const b = get().bubble;
        if (b && b.holdKey === holdKey && !b.released) set({ bubble: { ...b, released: true } });
    },
    clear: (id) => {
        if (get().bubble?.id === id) set({ bubble: null });
    },
}));
