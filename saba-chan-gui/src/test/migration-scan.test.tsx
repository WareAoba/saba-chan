import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AddInstanceMigration } from '../components/Modals/AddInstanceMigration';
import type { ModuleInfo } from '../types/daemon';

describe('AddInstanceMigration 폴더 스캔', () => {
    afterEach(() => {
        delete (window.api as Partial<typeof window.api>).openFolderDialog;
        delete (window.api as Partial<typeof window.api>).migrationScanDir;
        delete (window.api as unknown as Record<string, unknown>).moduleDetect;
    });

    it('scanDir 한 번의 결과(matches)로 모듈을 고른다', async () => {
        window.api.openFolderDialog = vi.fn().mockResolvedValue('/srv/old');
        window.api.migrationScanDir = vi.fn().mockResolvedValue({ ok: true, matches: [{ name: 'minecraft' }] });
        (window.api as unknown as Record<string, unknown>).moduleDetect = vi.fn();
        render(
            <AddInstanceMigration
                extensions={[{ name: 'minecraft', display_name: 'Minecraft Java' }] as ModuleInfo[]}
                servers={[]}
                onAddServer={vi.fn()}
                onBack={vi.fn()}
            />,
        );
        fireEvent.click(screen.getByRole('button', { name: 'Browse' }));
        expect(await screen.findByText(/Minecraft Java/)).toBeTruthy();
        expect(window.api.migrationScanDir).toHaveBeenCalledWith('/srv/old');
        expect((window.api as unknown as Record<string, unknown>).moduleDetect).not.toHaveBeenCalled();
    });

    it('스캔 뒤 경로를 고치면 옛 경로의 감지 결과를 지운다 (다른 폴더를 옛 모듈로 가져오지 않게)', async () => {
        window.api.openFolderDialog = vi.fn().mockResolvedValue('/srv/old');
        window.api.migrationScanDir = vi.fn().mockResolvedValue({ ok: true, matches: [{ name: 'minecraft' }] });
        render(
            <AddInstanceMigration
                extensions={[{ name: 'minecraft', display_name: 'Minecraft Java' }] as ModuleInfo[]}
                servers={[]}
                onAddServer={vi.fn()}
                onBack={vi.fn()}
            />,
        );
        fireEvent.click(screen.getByRole('button', { name: 'Browse' }));
        expect(await screen.findByText(/Minecraft Java/)).toBeTruthy();
        fireEvent.change(screen.getByDisplayValue('/srv/old'), { target: { value: '/srv/other' } });
        expect(screen.queryByText(/Minecraft Java/)).toBeNull();
    });

    it('그 자리에 연결할지 사바쨩 폴더로 옮길지 고르고, 고른 방식으로 가져온다', async () => {
        window.api.openFolderDialog = vi.fn().mockResolvedValue('/srv/old');
        window.api.migrationScanDir = vi.fn().mockResolvedValue({ ok: true, matches: [{ name: 'minecraft' }] });
        const onAddServer = vi.fn().mockResolvedValue(true);
        render(
            <AddInstanceMigration
                extensions={[{ name: 'minecraft', display_name: 'Minecraft Java' }] as ModuleInfo[]}
                servers={[]}
                onAddServer={onAddServer}
                onBack={vi.fn()}
            />,
        );
        fireEvent.click(screen.getByRole('button', { name: 'Browse' }));
        fireEvent.click(await screen.findByText('Next'));
        // 기본은 그 자리에 연결
        expect(screen.getByRole('radio', { name: /Keep current folder/ }).getAttribute('aria-checked')).toBe('true');
        fireEvent.click(screen.getByRole('radio', { name: /Move into Saba-chan folder/ }));
        fireEvent.click(screen.getByText('Import'));
        await vi.waitFor(() => expect(onAddServer).toHaveBeenCalled());
        expect(onAddServer.mock.calls[0][0]).toMatchObject({ migration_source: '/srv/old', migration_mode: 'move' });
    });

    // 사용자 11-08: 켜져 있는 서버는 불러오기를 누르기 전에 끄라고 보인다 (데몬 detect `notices`)
    it('폴더의 서버가 켜져 있으면 다음으로 가기 전에 경고하고, 불러오기 화면에서 바뀌는 것을 보인다', async () => {
        window.api.openFolderDialog = vi.fn().mockResolvedValue('/srv/old');
        window.api.migrationScanDir = vi
            .fn()
            .mockResolvedValue({ ok: true, matches: [{ name: 'minecraft' }], notices: ['server_running'] });
        render(
            <AddInstanceMigration
                extensions={[{ name: 'minecraft', display_name: 'Minecraft Java' }] as ModuleInfo[]}
                servers={[]}
                onAddServer={vi.fn()}
                onBack={vi.fn()}
            />,
        );
        fireEvent.click(screen.getByRole('button', { name: 'Browse' }));
        expect(await screen.findByText('This server is running')).toBeTruthy();
        fireEvent.click(screen.getByText('Next'));
        expect(screen.getByText('This server is running')).toBeTruthy();
        expect(screen.getByText('What changes on import')).toBeTruthy();
        expect(screen.getByText('Removing it from Saba-chan also deletes its backups')).toBeTruthy();
    });
});
