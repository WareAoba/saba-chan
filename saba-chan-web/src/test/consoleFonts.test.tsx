import { fireEvent, render, screen } from '@testing-library/react';
import { beforeAll, beforeEach, describe, expect, it } from 'vitest';
import { MoreTab } from '../components/MoreTab';
import { initI18n } from '../i18n';
import { readConsoleFonts } from '../prefs';

// 사용자 11-02: 웹 콘솔도 앱과 같은 글꼴을 글자마다 고른다. 이 브라우저에만 기억한다 (언어·테마처럼)
beforeAll(() => initI18n('ko'));
beforeEach(() => localStorage.clear());

describe('웹 콘솔 글꼴', () => {
    it('기본값은 앱과 같다', () => {
        expect(readConsoleFonts()).toEqual({ latin: 'cascadia-code', ko: 'd2coding', ja: 'hackgen', zh: 'sarasa-mono-sc' });
        localStorage.setItem('saba-web.consoleFonts', '{"ko":"hackgen","ja":"udev-gothic"');
        // 깨진 값은 기본값
        expect(readConsoleFonts().ko).toBe('d2coding');
    });

    it('화면 설정에서 고르면 기억하고 바로 그린다', async () => {
        render(<MoreTab api={{}} />);
        fireEvent.click(screen.getByRole('tab', { name: '이 화면' }));
        const ja = screen.getByRole('combobox', { name: '일본어' });
        fireEvent.click(ja);
        fireEvent.click(screen.getByRole('option', { name: 'UDEV Gothic' }));
        expect(readConsoleFonts().ja).toBe('udev-gothic');
        expect(document.documentElement.style.getPropertyValue('--font-mono')).toContain("'saba-ja-udev-gothic'");
    });
});
