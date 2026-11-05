/**
 * 원격 데몬 중계가 실패하면 데몬은 {error, error_code}만 준다 (success:false가 없다, src/remote_daemon/peer.rs).
 * 백업·예약 저장은 이것을 성공으로 보이지 않고 실패로 알린다.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import i18n from 'i18next';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MaintenanceTab } from '../components/servers/MaintenanceTab';
import type { Server } from '../stores/useServerStore';

const server = { id: 'srv1', status: 'stopped' } as unknown as Server;
const relayFailed = { error: 'unreachable', error_code: 'remote_unreachable' };
const API_KEYS = ['instanceScheduleGet', 'instanceScheduleSave', 'instanceBackups', 'instanceBackupCreate'];

afterEach(() => {
    for (const k of API_KEYS) delete (window.api as unknown as Record<string, unknown>)[k];
});

const mount = async () => {
    window.showToast = vi.fn();
    window.api.instanceScheduleGet = vi.fn().mockResolvedValue({
        success: true,
        schedule: [],
        backup_keep: 5,
        backup_supported: true,
        revision: 'r1',
    });
    window.api.instanceBackups = vi.fn().mockResolvedValue({ success: true, backups: [] });
    render(<MaintenanceTab server={server} />);
    const add = await screen.findByRole('button', { name: /Add task/ });
    await act(async () => {});
    return add;
};
const shown = () => vi.mocked(window.showToast!).mock.calls.map(([message, type]) => [message, type]);

describe('원격 중계 실패', () => {
    it('백업 만들기가 중계 오류를 받으면 성공 대신 오류를 알린다', async () => {
        await mount();
        window.api.instanceBackupCreate = vi.fn().mockResolvedValue(relayFailed);
        fireEvent.click(screen.getByRole('button', { name: i18n.t('gui:maintenance.backup_now') }));
        await waitFor(() => expect(shown()).toContainEqual(['unreachable', 'error']));
        expect(shown().some(([, type]) => type === 'success')).toBe(false);
    });

    it('예약 저장이 중계 오류를 받으면 저장했다고 하지 않고 오류를 보인다', async () => {
        const add = await mount();
        window.api.instanceScheduleSave = vi.fn().mockResolvedValue(relayFailed);
        fireEvent.click(add);
        await screen.findByRole('tablist', { name: 'Task 1 type' });
        fireEvent.click(screen.getByRole('button', { name: /Save/ }));
        await screen.findByText('unreachable');
        expect(shown()).not.toContainEqual([i18n.t('gui:maintenance.schedule_saved'), 'success']);
    });
});
