/**
 * 창 크기를 바꾸는 사이 웹뷰가 못 그린 자리는 셸이 칠한 바탕색이 보인다.
 * 테마를 적용하면 그 테마의 --bg-0을 셸에 알려야 흰 바탕이 비치지 않는다.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setTheme } from '../utils/themeManager';

describe('창 바탕색', () => {
    beforeEach(() => {
        window.api.windowSetBackground = vi.fn();
        document.documentElement.style.setProperty('--bg-0', '#e6e8ed');
    });
    afterEach(() => {
        delete document.documentElement.dataset.trayPopup;
        document.documentElement.style.removeProperty('--bg-0');
    });

    it('테마를 적용하면 그 테마의 바탕색을 셸에 알린다', () => {
        setTheme('light');
        expect(window.api.windowSetBackground).toHaveBeenCalledWith('#e6e8ed');
    });

    it('투명한 트레이 팝업은 바탕색을 알리지 않는다', () => {
        document.documentElement.dataset.trayPopup = '';
        setTheme('light');
        expect(window.api.windowSetBackground).not.toHaveBeenCalled();
    });
});
