import { afterEach, describe, expect, it, vi } from 'vitest';
import { flashSetting } from '../utils/findOnScreen';

describe('설정 칸 찾기', () => {
    afterEach(() => {
        vi.useRealTimers();
        document.body.innerHTML = '';
    });

    it('접힌(hidden) 고급 묶음 안의 칸은 펼쳐질 때까지 기다렸다가 반짝인다', () => {
        vi.useFakeTimers();
        document.body.innerHTML =
            '<div class="ss-fold" hidden><div class="ss-row"><span class="ss-name">Max players</span></div></div>';
        const fold = document.querySelector<HTMLElement>('.ss-fold') as HTMLElement;
        const row = document.querySelector('.ss-row') as HTMLElement;
        flashSetting('Max players');
        vi.advanceTimersByTime(300);
        expect(row.classList.contains('palette-found')).toBe(false);
        fold.hidden = false;
        vi.advanceTimersByTime(150);
        expect(row.classList.contains('palette-found')).toBe(true);
    });
});
