import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { DateField } from '../components/ui/DateTimePicker';
import { OklchPicker } from '../components/ui/OklchPicker';
import { clickOutside } from './outside';

// 날짜·색 선택 팝업도 공통 Popover(.saba-popover)에 뜬다: 여닫기·Esc·바깥 누르기가 다른 창과 같다
describe('선택 팝업 = 공통 Popover', () => {
    it('날짜 팝업은 공통 창에 뜨고 바깥을 누르면 닫힌다', async () => {
        render(<DateField value="2026-10-30" onChange={vi.fn()} ariaLabel="Date" />);
        const trigger = screen.getByRole('button', { name: 'Date' });
        fireEvent.click(trigger);
        expect(document.querySelector('.saba-popover.dtp-panel')).not.toBeNull();
        await clickOutside();
        expect(document.querySelector('.dtp-panel')).toBeNull();
    });

    it('색 팝업은 공통 창에 뜨고 Esc로 닫힌다', () => {
        render(<OklchPicker value="#ff8800" onChange={vi.fn()} label="Color" />);
        fireEvent.click(screen.getByRole('button', { name: 'Color' }));
        const pop = document.querySelector('.saba-popover.oklch-pop');
        expect(pop).not.toBeNull();
        fireEvent.keyDown(pop as Element, { key: 'Escape' });
        expect(document.querySelector('.oklch-pop')).toBeNull();
    });
});
