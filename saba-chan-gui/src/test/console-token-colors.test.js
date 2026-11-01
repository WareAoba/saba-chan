/**
 * 콘솔 구문 강조 색: 사용자가 고른 색은 테마 기본색을 덮고, 빈 값이면 기본색으로 돌아간다.
 */
import { describe, expect, it } from 'vitest';
import { applyCustomTheme, THEME_DEFAULTS } from '../utils/themeManager';

const tok = (name) => document.documentElement.style.getPropertyValue(`--tok-${name}`);

describe('콘솔 구문 강조 색', () => {
    it('고른 색만 덮고, 빈 값으로 바꾸면 기본색으로 돌아간다', () => {
        applyCustomTheme({ ...THEME_DEFAULTS, consoleTokenColors: { error: '#ff0000', warn: '' } });
        expect(tok('error')).toBe('#ff0000');
        expect(tok('warn')).toBe('');

        applyCustomTheme({ ...THEME_DEFAULTS, consoleTokenColors: { error: '' } });
        expect(tok('error')).toBe('');
    });
});
