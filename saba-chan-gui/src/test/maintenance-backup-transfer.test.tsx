/**
 * 백업 내보내기·가져오기: 줄의 "내보내기"는 그 백업을 zip 하나로 쓰게 하고,
 * "가져오기"는 고른 zip이 무엇인지(만든 서버·때·크기) 먼저 보인 뒤 확인하면 목록에 올린다.
 * 사바쨩 백업이 아닌 파일은 창을 띄우지 않고 이유를 알린다.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import i18n from 'i18next';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MaintenanceTab } from '../components/servers/MaintenanceTab';
import type { Server } from '../stores/useServerStore';
import * as helpers from '../utils/helpers';

const server = { id: 'srv1', name: '친구들 서버', status: 'stopped' } as unknown as Server;
const API_KEYS = ['instanceScheduleGet', 'instanceBackups', 'instanceBackupExport', 'instanceBackupImport'];
const tr = (key: string, vars?: Record<string, unknown>) => i18n.t(`gui:maintenance.${key}`, vars ?? {});

const mount = () => {
    window.api.instanceScheduleGet = vi.fn().mockResolvedValue({ success: true, schedule: [], backup_keep: 5, backup_supported: true, revision: 'r1' });
    window.api.instanceBackups = vi.fn().mockResolvedValue({
        success: true,
        total_bytes: 1024,
        backups: [
            { name: '20261112-040000-000-scheduled.zip', kind: 'scheduled', created: 1762920000, size: 1024, stored: 1024, readable: true },
            { name: '20261030-120000-000-imported.zip', kind: 'imported', created: 1761800000, size: 1024, stored: 0, readable: true, imported_from: '옛 서버' },
        ],
    });
    render(<MaintenanceTab server={server} />);
};

afterEach(() => {
    for (const k of API_KEYS) delete (window.api as unknown as Record<string, unknown>)[k];
    vi.restoreAllMocks();
});

describe('백업 내보내기·가져오기', () => {
    it('줄의 내보내기는 그 백업을 내보내고 쓴 자리와 크기를 알린다', async () => {
        const toast = vi.spyOn(helpers, 'safeShowToast').mockImplementation(() => null);
        window.api.instanceBackupExport = vi.fn().mockResolvedValue({ success: true, path: 'D:\\백업\\out.zip', size: 2048 });
        mount();
        const buttons = await screen.findAllByRole('button', { name: tr('backup_export') });
        fireEvent.click(buttons[0]);
        await waitFor(() => expect(toast).toHaveBeenCalled());
        expect(window.api.instanceBackupExport).toHaveBeenCalledWith('srv1', '20261112-040000-000-scheduled.zip', expect.stringMatching(/^친구들 서버-\d{4}-\d{2}-\d{2}\.zip$/));
        expect(toast.mock.calls[0][0]).toBe(tr('backup_exported', { path: 'D:\\백업\\out.zip', size: '2.0 KB' }));
        // 가져온 백업은 어느 서버에서 만든 것인지 줄에 보인다
        expect(screen.getByText(tr('backup_imported_from', { server: '옛 서버' }), { exact: false })).toBeTruthy();
    });

    it('가져오기는 무엇인지 먼저 보이고, 확인하면 같은 파일을 목록에 올린다', async () => {
        const toast = vi.spyOn(helpers, 'safeShowToast').mockImplementation(() => null);
        window.api.instanceBackupImport = vi
            .fn()
            .mockResolvedValueOnce({ success: true, path: 'E:\\usb.zip', preview: { server: '친구들 서버', module: 'minecraft', created: 1762920000, size: 2048 } })
            .mockResolvedValueOnce({ success: true, path: 'E:\\usb.zip', backup: { name: 'x-imported.zip' } });
        mount();
        fireEvent.click(await screen.findByRole('button', { name: tr('backup_import') }));
        const dialog = await screen.findByRole('dialog');
        expect(dialog.textContent).toContain('usb.zip');
        expect(dialog.textContent).toContain("친구들 서버 (minecraft)");
        expect(dialog.textContent).toContain('2.0 KB');
        expect(window.api.instanceBackupImport).toHaveBeenCalledWith('srv1', { preview: true });
        fireEvent.click(screen.getAllByRole('button', { name: tr('backup_import') }).find((b) => dialog.contains(b))!);
        await waitFor(() => expect(window.api.instanceBackupImport).toHaveBeenLastCalledWith('srv1', { path: 'E:\\usb.zip' }));
        await waitFor(() => expect(toast).toHaveBeenCalledWith(tr('backup_imported'), 'success', expect.anything()));
        await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    });

    it('사바쨩 백업이 아닌 파일을 고르면 창을 띄우지 않고 이유를 알린다', async () => {
        const toast = vi.spyOn(helpers, 'safeShowToast').mockImplementation(() => null);
        window.api.instanceBackupImport = vi.fn().mockResolvedValue({ success: false, error: 'this file is not an exported saba-chan backup', error_code: 'import_not_backup', path: 'E:\\x.zip' });
        mount();
        fireEvent.click(await screen.findByRole('button', { name: tr('backup_import') }));
        await waitFor(() => expect(toast).toHaveBeenCalledWith(tr('err_import_not_backup'), 'error', expect.anything()));
        expect(screen.queryByRole('dialog')).toBeNull();
    });
});
