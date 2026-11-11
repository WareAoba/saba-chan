import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { SabaSlider } from '../components/ui/SabaUI';

// 글꼴 크기 막대 두 곳이 따로 그리던 range 칸을 공통 부품 하나로 (고아 컨트롤 정리)
describe('SabaSlider', () => {
    it('채운 정도를 값으로 칠하고 바꾸면 숫자로 알린다', () => {
        const onChange = vi.fn();
        render(<SabaSlider aria-label="Size" min={50} max={200} step={5} value={125} onChange={onChange} />);
        const el = screen.getByRole('slider', { name: 'Size' });
        expect(el.classList).toContain('saba-slider');
        expect(el.style.getPropertyValue('--fill')).toBe('50%');
        fireEvent.change(el, { target: { value: '150' } });
        expect(onChange).toHaveBeenCalledWith(150);
    });

    it('설정 화면은 직접 range 칸을 그리지 않는다', () => {
        for (const f of ['settings/AppearanceTab.tsx', 'settings/ConsoleTab.tsx', 'WindowBackdropSetting.tsx']) {
            const src = readFileSync(join(__dirname, '../components/Modals', f), 'utf8');
            expect(src, f).not.toMatch(/type="range"/);
        }
    });
});
