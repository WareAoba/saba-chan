/**
 * 백업 목록은 줄마다 크기를 보이지 않고, 데몬이 준 백업 전체 용량 하나만 목록 위에 보인다.
 * 복원 직전 백업은 서버를 켠 뒤 15분이 지나면 지워진다고 줄에 알린다.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import i18n from 'i18next';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MaintenanceTab } from '../components/servers/MaintenanceTab';
import type { Server } from '../stores/useServerStore';
import type { ApiBody } from '../types/daemon';

const server = { id: 'srv1', status: 'stopped' } as unknown as Server;
const API_KEYS = ['instanceScheduleGet', 'instanceBackups'];

afterEach(() => {
    for (const k of API_KEYS) delete (window.api as unknown as Record<string, unknown>)[k];
});

describe('백업 목록 용량', () => {
    it('전체 용량 하나만 보이고 줄마다 크기를 쓰지 않는다', async () => {
        window.api.instanceScheduleGet = vi.fn().mockResolvedValue({ success: true, schedule: [], backup_keep: 5, backup_supported: true, revision: 'r1' });
        window.api.instanceBackups = vi.fn().mockResolvedValue({
            success: true,
            total_bytes: 38 * 1024 * 1024,
            backups: [
                { name: '20261109-164229-693-pre-restore.zip', kind: 'pre-restore', created: 1762674149, size: 11 * 1024 * 1024, stored: 4 * 1024 * 1024, readable: true },
                { name: '20261109-164142-262-manual.zip', kind: 'manual', created: 1762674102, size: 20 * 1024 * 1024, stored: 12 * 1024 * 1024, readable: true },
            ],
        });
        render(<MaintenanceTab server={server} />);
        await screen.findByText(i18n.t('gui:maintenance.backup_total', { size: '38 MB', count: 2 }));
        await act(async () => {});
        expect(screen.queryByText(/11 MB/)).toBeNull();
        expect(screen.queryByText(/20 MB/)).toBeNull();
        expect(screen.getByText(i18n.t('gui:maintenance.backup_pre_restore_hint'), { exact: false })).toBeTruthy();
    });

    // 데몬이 backups_unreadable을 주면 "백업 없음"이 아니라 오류와 다시 시도 버튼을 보인다
    it('목록을 읽지 못하면 빈 목록 대신 오류와 다시 시도를 보이고, 다시 시도하면 불러온다', async () => {
        window.api.instanceScheduleGet = vi.fn().mockResolvedValue({ success: true, schedule: [], backup_keep: 5, backup_supported: true, revision: 'r1' });
        window.api.instanceBackups = vi
            .fn()
            .mockResolvedValueOnce({ success: false, error: 'Could not read the backup folder', error_code: 'backups_unreadable' })
            .mockResolvedValueOnce({ success: true, total_bytes: 1024, backups: [{ name: '20261109-164142-262-manual.zip', kind: 'manual', created: 1762674102, size: 1024, stored: 1024, readable: true }] });
        render(<MaintenanceTab server={server} />);
        const retry = await screen.findByRole('button', { name: i18n.t('gui:maintenance.retry') });
        expect(screen.queryByText(i18n.t('gui:maintenance.backups_empty'))).toBeNull();
        // 데몬의 영문 오류 문구가 아니라 화면 언어의 문구를 보인다
        expect(screen.getByText(i18n.t('gui:maintenance.err_backups_unreadable'))).toBeTruthy();
        expect(screen.queryByText('Could not read the backup folder')).toBeNull();
        fireEvent.click(retry);
        await waitFor(() => expect(screen.getByText(/manual\.zip/)).toBeTruthy());
    });

    it('목록 요청이 예외로 실패해도 빈 목록으로 보이지 않는다', async () => {
        window.api.instanceScheduleGet = vi.fn().mockResolvedValue({ success: true, schedule: [], backup_keep: 5, backup_supported: true, revision: 'r1' });
        window.api.instanceBackups = vi.fn().mockRejectedValue(new Error('network'));
        render(<MaintenanceTab server={server} />);
        await screen.findByRole('button', { name: i18n.t('gui:maintenance.retry') });
        expect(screen.queryByText(i18n.t('gui:maintenance.backups_empty'))).toBeNull();
    });

    // 사용자 11-17: 탭을 오가도 백업 목록을 다시 불러오는 동안 비워 두지 않는다
    it('다시 열면 받아 둔 목록을 바로 보이고 새 목록은 뒤에서 받는다', async () => {
        const cached = { id: 'srv-cache', status: 'stopped' } as unknown as Server;
        window.api.instanceScheduleGet = vi.fn().mockResolvedValue({ success: true, schedule: [], backup_keep: 5, backup_supported: true, revision: 'r1' });
        window.api.instanceBackups = vi.fn().mockResolvedValue({
            success: true,
            total_bytes: 1024,
            backups: [{ name: '20261109-164142-262-manual.zip', kind: 'manual', created: 1762674102, size: 1024, stored: 1024, readable: true }],
        });
        const first = render(<MaintenanceTab server={cached} />);
        await screen.findByText(/manual\.zip/);
        first.unmount();

        window.api.instanceBackups = vi.fn(() => new Promise<ApiBody>(() => {}));
        render(<MaintenanceTab server={cached} />);
        expect(screen.getByText(/manual\.zip/)).toBeTruthy();
        expect(window.api.instanceBackups).toHaveBeenCalledWith('srv-cache');
    });

    // 받아 둔 예약 정보로 먼저 보인 뒤 새 정보가 와도, 일정이 그대로면 그사이 고친 내용은 지우지 않는다
    it('다시 연 뒤 새 정보가 오기 전에 고친 보관 개수는 같은 일정이 오면 남는다', async () => {
        const cached = { id: 'srv-draft', status: 'stopped' } as unknown as Server;
        const info = { success: true, schedule: [], backup_keep: 5, backup_supported: true, revision: 'r1' };
        window.api.instanceBackups = vi.fn().mockResolvedValue({ success: true, total_bytes: 0, backups: [] });
        window.api.instanceScheduleGet = vi.fn().mockResolvedValue(info);
        const first = render(<MaintenanceTab server={cached} />);
        await screen.findByDisplayValue('5');
        first.unmount();

        let answer: (v: ApiBody) => void = () => {};
        window.api.instanceScheduleGet = vi.fn(() => new Promise<ApiBody>((r) => { answer = r; }));
        render(<MaintenanceTab server={cached} />);
        fireEvent.change(screen.getByDisplayValue('5'), { target: { value: '7' } });
        await act(async () => answer({ ...info, next_runs: [] }));
        expect(screen.getByDisplayValue('7')).toBeTruthy();
    });

    it('첫 응답 전에 떠났다 돌아와도 같은 응답이 오면 로딩에 머물지 않는다', async () => {
        const fresh = { id: 'srv-left', status: 'stopped' } as unknown as Server;
        const info = { success: true, schedule: [], backup_keep: 5, backup_supported: true, revision: 'r1' };
        window.api.instanceBackups = vi.fn().mockResolvedValue({ success: true, total_bytes: 0, backups: [] });
        const answers: ((v: ApiBody) => void)[] = [];
        window.api.instanceScheduleGet = vi.fn(() => new Promise<ApiBody>((r) => { answers.push(r); }));
        const first = render(<MaintenanceTab server={fresh} />);
        first.unmount();
        render(<MaintenanceTab server={fresh} />);
        await act(async () => answers[0](info));
        await act(async () => answers[1](info));
        expect(screen.getByDisplayValue('5')).toBeTruthy();
    });
});
