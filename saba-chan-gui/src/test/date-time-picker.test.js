import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { DateField, TimeField } from '../components/ui/DateTimePicker';

// 날짜·시간 칸은 OS 기본 달력·시계 대신 앱 팝업으로 고른다 (값 형식은 input과 같은 YYYY-MM-DD / HH:MM)
const panel = (trigger) => document.getElementById(trigger.getAttribute('aria-controls'));
const day = (trigger, date) => panel(trigger).querySelector(`[data-date="${date}"]`);

describe('DateField', () => {
    it('누르면 그 달 달력이 열리고, 날짜를 누르면 값이 바뀌고 닫힌다', () => {
        const onChange = vi.fn();
        render(<DateField value="2026-10-30" onChange={onChange} ariaLabel="Date" />);
        const trigger = screen.getByRole('button', { name: 'Date' });
        fireEvent.click(trigger);
        expect(day(trigger, '2026-10-30').getAttribute('aria-pressed')).toBe('true');
        fireEvent.click(day(trigger, '2026-10-12'));
        expect(onChange).toHaveBeenCalledWith('2026-10-12');
        expect(trigger.getAttribute('aria-expanded')).toBe('false');
    });

    it('달을 넘기고, min·max 밖 날짜는 고를 수 없다', () => {
        const onChange = vi.fn();
        render(<DateField value="2026-10-30" min="2026-10-05" max="2026-11-03" onChange={onChange} ariaLabel="Date" />);
        const trigger = screen.getByRole('button', { name: 'Date' });
        fireEvent.click(trigger);
        expect(day(trigger, '2026-10-04').disabled).toBe(true);
        fireEvent.click(screen.getByRole('button', { name: 'Next month' }));
        expect(day(trigger, '2026-11-04').disabled).toBe(true);
        fireEvent.click(day(trigger, '2026-11-02'));
        expect(onChange).toHaveBeenCalledWith('2026-11-02');
    });

    it('방향키로 날짜를 옮기고 Enter로 고르며, 지우기는 clearable일 때만 있다', () => {
        const onChange = vi.fn();
        const { rerender } = render(<DateField value="2026-10-31" onChange={onChange} ariaLabel="Date" />);
        const trigger = screen.getByRole('button', { name: 'Date' });
        fireEvent.click(trigger);
        expect(screen.queryByRole('button', { name: 'Clear' })).toBeNull();
        fireEvent.keyDown(document.activeElement, { key: 'ArrowRight' });
        expect(document.activeElement.dataset.date).toBe('2026-11-01');
        fireEvent.click(document.activeElement);
        expect(onChange).toHaveBeenCalledWith('2026-11-01');
        rerender(<DateField value="2026-10-31" onChange={onChange} ariaLabel="Date" clearable />);
        fireEvent.click(trigger);
        fireEvent.click(screen.getByRole('button', { name: 'Clear' }));
        expect(onChange).toHaveBeenLastCalledWith('');
    });
});

describe('TimeField', () => {
    it('시와 분을 따로 고른다 (24시간)', () => {
        const onChange = vi.fn();
        const { rerender } = render(<TimeField value="04:30" onChange={onChange} ariaLabel="Time" />);
        const trigger = screen.getByRole('button', { name: 'Time' });
        expect(trigger.textContent).toContain('04:30');
        fireEvent.click(trigger);
        const p = panel(trigger);
        fireEvent.click(p.querySelector('[data-hour="17"]'));
        expect(onChange).toHaveBeenLastCalledWith('17:30');
        rerender(<TimeField value="17:30" onChange={onChange} ariaLabel="Time" />);
        fireEvent.click(p.querySelector('[data-minute="05"]'));
        expect(onChange).toHaveBeenLastCalledWith('17:05');
        expect(trigger.getAttribute('aria-expanded')).toBe('false');
    });
});

describe('작업 예약·감사 기록', () => {
    it('OS 기본 날짜·시간 입력을 쓰지 않는다', () => {
        for (const f of ['MaintenanceTab.js', 'AuditTab.js']) {
            const src = readFileSync(join(__dirname, '../components/servers', f), 'utf8');
            expect(src).not.toMatch(/type="(date|time)"/);
        }
    });
});
