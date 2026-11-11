import { act, render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import Toast from '../components/Toast/Toast';

describe('Toast 종류 아이콘', () => {
    it.each(['success', 'error', 'warning', 'info'] as const)('%s 토스트는 앱 아이콘(svg)을 앞에 단다', (type) => {
        const { container } = render(<Toast />);
        act(() => {
            window.showToast!('메시지', type, 3000);
        });
        const icon = container.querySelector(`.toast-${type} .toast-icon`);
        expect(icon?.classList.contains(`tone-${type}`)).toBe(true);
        expect(icon?.querySelector('svg')).toBeTruthy();
    });
});

describe('토스트 문구', () => {
    it('종류 아이콘과 겹치는 이모지로 시작하지 않는다 (모든 언어)', async () => {
        const { readdirSync, readFileSync } = await import('node:fs');
        const { join, resolve } = await import('node:path');
        const dir = resolve(__dirname, '../../../locales');
        const emoji = /^\s*[☀-➿\u{1F300}-\u{1FAFF}]/u;
        const bad: string[] = [];
        const walk = (o: Record<string, unknown>, path: string, lang: string) => {
            for (const [k, v] of Object.entries(o)) {
                if (v && typeof v === 'object') walk(v as Record<string, unknown>, `${path}${k}.`, lang);
                else if (
                    typeof v === 'string' &&
                    /toast|_title$|unreadable|jar_path_updated|install_completed/.test(k) &&
                    emoji.test(v)
                )
                    bad.push(`${lang}:${path}${k}`);
            }
        };
        for (const lang of readdirSync(dir))
            walk(JSON.parse(readFileSync(join(dir, lang, 'gui.json'), 'utf8')), '', lang);
        expect(bad).toEqual([]);
    });
});
