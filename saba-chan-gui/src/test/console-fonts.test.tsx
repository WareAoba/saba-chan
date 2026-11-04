import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import i18n from 'i18next';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SettingsModalImpl from '../components/Modals/SettingsModal';
import { useSettingsStore } from '../stores/useSettingsStore';
import {
    CONSOLE_FONT_CHOICES,
    CONSOLE_FONT_DEFAULTS,
    consoleFontStack,
    normalizeConsoleFonts,
} from '../utils/consoleFonts';
import { openDropdown } from './dropdown';
import type { ComponentProps, ComponentType } from 'react';

/** 시험에 필요한 props만 넘기려고 느슨하게 받는다 */
const SettingsModal = SettingsModalImpl as ComponentType<Partial<ComponentProps<typeof SettingsModalImpl>>>;

// 사용자 11-02: 콘솔 글꼴을 글자마다 고른다. 기본은 영문 Cascadia, 한글 D2Coding, 일본어 HackGen
describe('콘솔 글꼴', () => {
    it('기본값과 글자별 후보', () => {
        expect(CONSOLE_FONT_DEFAULTS).toEqual({
            latin: 'cascadia-code',
            ko: 'd2coding',
            ja: 'hackgen',
            zh: 'sarasa-mono-sc',
        });
        expect(CONSOLE_FONT_CHOICES.ko).toEqual(['d2coding', 'nanum-gothic-coding', 'neodgm-code']);
        expect(CONSOLE_FONT_CHOICES.ja).toEqual(['sarasa-mono-j', 'udev-gothic', 'hackgen', 'm-plus-1-code']);
        expect(CONSOLE_FONT_CHOICES.zh).toEqual(['sarasa-mono-sc', 'sarasa-mono-tc']);
        // 영문은 모든 후보에서
        expect(CONSOLE_FONT_CHOICES.latin).toHaveLength(10);
    });

    it('그 글자의 후보가 아니면 기본값으로 그린다', () => {
        expect(normalizeConsoleFonts({ ko: 'hackgen', ja: 'udev-gothic' })).toEqual({
            ...CONSOLE_FONT_DEFAULTS,
            ja: 'udev-gothic',
        });
        expect(normalizeConsoleFonts(undefined)).toEqual(CONSOLE_FONT_DEFAULTS);
    });

    it('글자 순서: 영문, 한글, 한자는 화면 언어가 중국어일 때만 중국어 글꼴 먼저', () => {
        expect(consoleFontStack(CONSOLE_FONT_DEFAULTS, 'ko')).toBe(
            "'saba-latin-cascadia-code', 'saba-ko-d2coding', 'saba-ja-hackgen', 'saba-zh-sarasa-mono-sc', monospace",
        );
        expect(consoleFontStack({ ...CONSOLE_FONT_DEFAULTS, zh: 'sarasa-mono-tc' }, 'zh-TW')).toBe(
            "'saba-latin-cascadia-code', 'saba-ko-d2coding', 'saba-zh-sarasa-mono-tc', 'saba-ja-hackgen', monospace",
        );
    });

    it('고를 수 있는 글꼴마다 글꼴 파일과 @font-face 가 있다', () => {
        const styles = join(__dirname, '..', 'styles');
        const css = readFileSync(join(styles, 'consoleFonts.css'), 'utf8');
        for (const [slot, ids] of Object.entries(CONSOLE_FONT_CHOICES)) {
            for (const id of ids) {
                const face = css.match(new RegExp(`font-family: 'saba-${slot}-${id}';[^}]*url\\('([^']+)'\\)`));
                expect(face, `saba-${slot}-${id}`).toBeTruthy();
                expect(existsSync(join(styles, (face as RegExpMatchArray)[1] as string)), (face as RegExpMatchArray)[1]).toBe(true);
            }
        }
        // 기본 글꼴 묶음은 첫 화면부터 (설정을 읽기 전)
        expect(readFileSync(join(styles, 'tokens.css'), 'utf8')).toContain(
            `--font-mono: ${consoleFontStack(CONSOLE_FONT_DEFAULTS)};`,
        );
    });
});

describe('설정 > 화면 > 콘솔 글꼴', () => {
    beforeEach(() => useSettingsStore.getState()._resetForTest());

    it('글자마다 드롭다운이 있고, 고르면 그 칸만 바뀐다', async () => {
        render(<SettingsModal isOpen onClose={vi.fn()} />);
        fireEvent.click(screen.getByRole('button', { name: i18n.t('gui:settings_modal.appearance') }));
        const box = screen.getByText(i18n.t('gui:settings_modal.console_fonts_label')).closest('.setting-item');
        const ko = within(box as HTMLElement).getByRole('combobox', { name: i18n.t('gui:settings_modal.console_fonts_ko') });
        // 한글 칸은 한글 글꼴만
        const options = within(openDropdown(ko))
            .getAllByRole('option')
            .map((o) => o.textContent);
        expect(options).toEqual(['D2Coding', '나눔고딕코딩', '네오둥근모 코드']);
        fireEvent.click(screen.getByRole('option', { name: '네오둥근모 코드' }));
        await act(async () => {});
        expect(useSettingsStore.getState().consoleFonts).toEqual({ ...CONSOLE_FONT_DEFAULTS, ko: 'neodgm-code' });
    });
});
