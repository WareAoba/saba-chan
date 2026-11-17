import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { BackupsPanel } from '../components/BackupsPanel';
import { initI18n } from '../i18n';
import { useApp } from '../store';
import { mockApi } from './mocks';

// 감사 cli-web P2: 링크를 되살리지 못한 복원도 "복원됨"만 보였고, 열 수 없는 백업도 복원 버튼이 눌렸다 (앱 MaintenanceBackups와 같게)
const flush = () => act(async () => {});

describe('웹 백업 복원', () => {
    beforeAll(() => initI18n('en'));
    afterEach(() => {
        cleanup();
        useApp.setState({ confirm: null, toasts: [] });
    });

    const mount = async (
        backups: object[],
        restoreRes: object = { success: true },
        createRes: object = { success: true },
    ) => {
        const api = {
            instanceBackups: vi.fn().mockResolvedValue({ success: true, backups }),
            instanceBackupRestore: vi.fn().mockResolvedValue(restoreRes),
            instanceBackupCreate: vi.fn().mockResolvedValue(createRes),
        };
        render(<BackupsPanel api={mockApi(api)} server={{ id: 's1' }} running={false} />);
        await flush();
        return api;
    };
    const restoreButtons = () => screen.getAllByRole('button', { name: 'Restore' });

    it('링크 일부를 되살리지 못했으면 앱과 같은 문구로 경고한다', async () => {
        await mount([{ name: 'a.zip', created: 1, kind: 'manual', size: 1, readable: true }], {
            success: true,
            links_not_restored: 2,
        });
        fireEvent.click(restoreButtons()[0]);
        await act(async () => useApp.getState().confirm?.done(true));
        await flush();
        expect(useApp.getState().toasts.at(-1)).toMatchObject({
            message: expect.stringMatching(/2 link/),
            tone: 'warn',
        });
    });

    it('열 수 없는 백업은 표시하고 복원 버튼을 끈다', async () => {
        await mount([
            { name: 'ok.zip', created: 2, kind: 'manual', size: 1, readable: true },
            { name: 'bad.zip', created: 1, kind: 'manual', size: 1, readable: false },
        ]);
        expect(screen.getAllByText(/cannot be opened/)).toHaveLength(1);
        expect(restoreButtons().map((b) => (b as HTMLButtonElement).disabled)).toEqual([false, true]);
    });

    // 백업 오류는 데몬 원문(OS 문구) 대신 앱과 같은 코드별 문구로 보인다
    it('백업 오류는 앱과 같은 코드별 문구로 보인다', async () => {
        await mount([], undefined, {
            success: false,
            error: 'No space left on device (os error 28)',
            error_code: 'backup_disk_full',
        });
        fireEvent.click(screen.getByRole('button', { name: /Back up now/ }));
        await flush();
        expect(useApp.getState().toasts.at(-1)).toMatchObject({
            message: 'Backup failed: not enough disk space. Free up space and try again.',
            tone: 'error',
        });
    });

    it('목록을 읽지 못한 오류도 코드별 문구로 보인다', async () => {
        const api = {
            instanceBackups: vi
                .fn()
                .mockResolvedValue({ success: false, error: 'EACCES', error_code: 'backups_unreadable' }),
        };
        render(<BackupsPanel api={mockApi(api)} server={{ id: 's1' }} running={false} />);
        await flush();
        expect(useApp.getState().toasts.at(-1)?.message).toMatch(/^Backup folder read failed/);
    });
});
