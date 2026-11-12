/**
 * 작업 기록 화면의 "다가오는 예약": 데몬이 모아 준 모든 서버의 다음 예약을 시각 순서로 보이고,
 * 겹치는 무거운 작업을 경고하며, 줄을 누르면 그 서버의 백업·예약 탭을 연다.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { UpcomingSchedules } from '../components/servers/UpcomingSchedules';
import i18n from '../i18n';
import { type Server, useServerStore } from '../stores/useServerStore';
import { useUIStore } from '../stores/useUIStore';
import { pickOption } from './dropdown';

const t = (k: string, o?: Record<string, unknown>) => i18n.t(`gui:${k}`, o) as string;

afterEach(() => {
    delete (window.api as Partial<typeof window.api>).schedulesUpcoming;
});

describe('다가오는 예약', () => {
    it('모든 서버의 예약을 보이고 겹치는 작업을 경고하며, 줄을 누르면 그 서버의 예약 탭으로 간다', async () => {
        const now = Math.floor(Date.now() / 1000);
        useServerStore.setState({ servers: [{ id: 'a', name: 'Friends' } as Server, { id: 'b', name: 'Mods' } as Server] });
        window.api.schedulesUpcoming = vi.fn(async () => ({
            success: true,
            upcoming: [
                { instance_id: 'a', instance: 'Friends', index: 1, next: now + 600, clash: false, task: { action: 'announce', at: '23:50', message: 'Restarting soon' } },
                { instance_id: 'a', instance: 'Friends', index: 0, next: now + 3600, clash: true, task: { action: 'backup', at: '04:00' } },
                { instance_id: 'b', instance: 'Mods', index: 0, next: now + 3700, clash: true, task: { action: 'restart', at: '04:01', weekdays: ['tue', 'fri'] } },
            ],
            clashes: [{ at: now + 3600, actions: ['backup', 'restart'] }],
        }));
        render(<UpcomingSchedules />);
        const rows = await screen.findAllByRole('button', { name: /Friends|Mods/ });
        expect(rows.map((r) => r.textContent)).toEqual([
            expect.stringContaining('Restarting soon'),
            expect.stringContaining(`${t('maintenance.action_backup')} · ${t('maintenance.mode_daily')}`),
            expect.stringMatching(/Tue.*Fri/),
        ]);
        expect(rows.map((r) => r.classList.contains('clash'))).toEqual([false, true, true]);
        expect(screen.getByRole('alert').textContent).toContain(
            `${t('maintenance.action_backup')}, ${t('maintenance.action_restart')}`,
        );
        expect(window.api.schedulesUpcoming).toHaveBeenLastCalledWith(24);

        fireEvent.click(rows[2]);
        expect(useUIStore.getState()).toMatchObject({ activeTab: 'servers', activeServerId: 'b', requestedServerTab: 'maintenance' });

        pickOption(screen.getByRole('combobox', { name: t('audit.upcoming_range') }), t('audit.upcoming_range_168'));
        await waitFor(() => expect(window.api.schedulesUpcoming).toHaveBeenLastCalledWith(168));
    });
});
