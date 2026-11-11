import { describe, expect, it } from 'vitest';
import { initTheme } from '../utils/themeManager';

describe('테마는 다른 창에서 바꿔도 따라간다', () => {
    it('메인 창이 테마를 바꾸면 바탕화면 펫·트레이 창도 다시 칠한다', () => {
        localStorage.setItem('saba-chan-theme', 'dark');
        initTheme();
        expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
        // 다른 창의 저장은 이 창에 storage 이벤트로만 온다
        localStorage.setItem('saba-chan-theme', 'light');
        window.dispatchEvent(new StorageEvent('storage', { key: 'saba-chan-theme', newValue: 'light' }));
        expect(document.documentElement.getAttribute('data-theme')).toBe('light');
    });
});
