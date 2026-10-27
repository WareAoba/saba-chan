/**
 * 이미 지난 날짜의 한 번 작업은 데몬이 저장은 하되 돌지 않으므로 `warnings`(date_passed)로 알린다.
 * GUI는 저장했다는 알림과 함께 몇 번 작업인지 경고 토스트로 보인다.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import i18n from 'i18next';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MaintenanceTab } from '../components/servers/MaintenanceTab';

const server = { id: 'srv1', status: 'stopped' };

afterEach(() => {
    delete window.api.instanceScheduleGet;
    delete window.api.instanceScheduleSave;
});

describe('지난 날짜의 한 번 작업', () => {
    it('저장 응답의 date_passed 경고를 작업 번호와 함께 경고 토스트로 보인다', async () => {
        window.showToast = vi.fn();
        window.api.instanceScheduleGet = vi.fn().mockResolvedValue({
            success: true,
            schedule: [],
            backup_keep: 5,
            backup_supported: false,
            announce_supported: false,
            revision: 'r1',
        });
        window.api.instanceScheduleSave = vi.fn().mockResolvedValue({
            success: true,
            warnings: [{ code: 'date_passed', task: 0 }],
        });
        render(<MaintenanceTab server={server} />);
        const add = await screen.findByRole('button', { name: /Add task/ });
        await act(async () => {});
        fireEvent.click(add);
        await screen.findByRole('tablist', { name: 'Task 1 type' });
        fireEvent.click(screen.getByRole('button', { name: /Save/ }));
        const passed = i18n.t('gui:maintenance.schedule_date_passed', { n: 1 });
        expect(passed).not.toBe('maintenance.schedule_date_passed');
        const shown = () => window.showToast.mock.calls.map(([message, type]) => [message, type]);
        await waitFor(() => expect(shown()).toContainEqual([passed, 'warning']));
        expect(shown()).toContainEqual([i18n.t('gui:maintenance.schedule_saved'), 'success']);
    });
});
