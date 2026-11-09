/**
 * 백업 목록은 줄마다 크기를 보이지 않고, 데몬이 준 백업 전체 용량 하나만 목록 위에 보인다.
 * 복원 직전 백업은 서버를 켠 뒤 15분이 지나면 지워진다고 줄에 알린다.
 */
import { act, render, screen } from '@testing-library/react';
import i18n from 'i18next';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MaintenanceTab } from '../components/servers/MaintenanceTab';
import type { Server } from '../stores/useServerStore';

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
});
