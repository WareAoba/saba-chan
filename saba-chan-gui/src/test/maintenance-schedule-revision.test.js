/**
 * 예약 작업 저장은 읽을 때 받은 revision을 함께 보낸다. 그사이 다른 곳에서 바뀌어 데몬이
 * 409 schedule_changed로 거절하면, 고친 내용은 그대로 두고 이유와 '다시 읽기'를 보인다 (L51).
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import i18n from 'i18next';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MaintenanceTab } from '../components/servers/MaintenanceTab';

const server = { id: 'srv1', status: 'stopped' };
const read = (revision) => ({
    success: true,
    schedule: [],
    backup_keep: 5,
    backup_supported: false,
    announce_supported: false,
    revision,
});

afterEach(() => {
    delete window.api.instanceScheduleGet;
    delete window.api.instanceScheduleSave;
});

describe('예약 작업 revision', () => {
    it('읽을 때 받은 revision을 싣고, schedule_changed면 이유를 보이고 다시 읽게 한다', async () => {
        window.showToast = vi.fn();
        window.api.instanceScheduleGet = vi.fn().mockResolvedValueOnce(read('r1')).mockResolvedValue(read('r2'));
        window.api.instanceScheduleSave = vi.fn().mockResolvedValue({
            success: false,
            error: 'The schedule was changed elsewhere; read it again',
            error_code: 'schedule_changed',
        });
        render(<MaintenanceTab server={server} />);
        const add = await screen.findByRole('button', { name: /Add task/ });
        // 읽은 일정으로 초안을 채우는 effect가 끝난 뒤에 고친다
        await act(async () => {});
        fireEvent.click(add);
        await screen.findByRole('tablist', { name: 'Task 1 type' });
        fireEvent.click(screen.getByRole('button', { name: /Save/ }));
        const changed = i18n.t('gui:maintenance.err_schedule_changed');
        expect(changed).not.toBe('maintenance.err_schedule_changed');
        await waitFor(() => expect(window.api.instanceScheduleSave).toHaveBeenCalled());
        await screen.findByText(changed);
        expect(window.api.instanceScheduleSave).toHaveBeenCalledWith('srv1', {
            schedule: [{ action: 'restart', enabled: true, every_minutes: 60 }],
            revision: 'r1',
        });
        // 고친 내용은 남아 있다
        expect(screen.getByRole('tablist', { name: 'Task 1 type' })).toBeTruthy();
        fireEvent.click(screen.getByRole('button', { name: i18n.t('gui:maintenance.reload') }));
        await waitFor(() => expect(window.api.instanceScheduleGet).toHaveBeenCalledTimes(2));
        await waitFor(() => expect(screen.queryByText(changed)).toBeNull());
    });
});
