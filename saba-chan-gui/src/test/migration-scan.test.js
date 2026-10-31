import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AddInstanceMigration } from '../components/Modals/AddInstanceMigration';

describe('AddInstanceMigration 폴더 스캔', () => {
    afterEach(() => {
        delete window.api.openFolderDialog;
        delete window.api.migrationScanDir;
        delete window.api.moduleDetect;
    });

    it('scanDir 한 번의 결과(matches)로 모듈을 고른다', async () => {
        window.api.openFolderDialog = vi.fn().mockResolvedValue('/srv/old');
        window.api.migrationScanDir = vi.fn().mockResolvedValue({ ok: true, matches: [{ name: 'minecraft' }] });
        window.api.moduleDetect = vi.fn();
        render(
            <AddInstanceMigration
                extensions={[{ name: 'minecraft', display_name: 'Minecraft Java' }]}
                servers={[]}
                onAddServer={vi.fn()}
                onStepChange={vi.fn()}
                onBack={vi.fn()}
                onClose={vi.fn()}
            />,
        );
        fireEvent.click(screen.getByRole('button', { name: 'Browse' }));
        expect(await screen.findByText(/Minecraft Java/)).toBeTruthy();
        expect(window.api.migrationScanDir).toHaveBeenCalledWith('/srv/old');
        expect(window.api.moduleDetect).not.toHaveBeenCalled();
    });

    it('스캔 뒤 경로를 고치면 옛 경로의 감지 결과를 지운다 (다른 폴더를 옛 모듈로 가져오지 않게)', async () => {
        window.api.openFolderDialog = vi.fn().mockResolvedValue('/srv/old');
        window.api.migrationScanDir = vi.fn().mockResolvedValue({ ok: true, matches: [{ name: 'minecraft' }] });
        render(
            <AddInstanceMigration
                extensions={[{ name: 'minecraft', display_name: 'Minecraft Java' }]}
                servers={[]}
                onAddServer={vi.fn()}
                onStepChange={vi.fn()}
                onBack={vi.fn()}
                onClose={vi.fn()}
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
                extensions={[{ name: 'minecraft', display_name: 'Minecraft Java' }]}
                servers={[]}
                onAddServer={onAddServer}
                onStepChange={vi.fn()}
                onBack={vi.fn()}
                onClose={vi.fn()}
            />,
        );
        fireEvent.click(screen.getByRole('button', { name: 'Browse' }));
        fireEvent.click(await screen.findByText('Next'));
        // 기본은 그 자리에 연결
        expect(screen.getByRole('tab', { name: 'Keep it where it is' }).getAttribute('aria-selected')).toBe('true');
        fireEvent.click(screen.getByRole('tab', { name: 'Move into saba-chan' }));
        fireEvent.click(screen.getByText('Import'));
        await vi.waitFor(() => expect(onAddServer).toHaveBeenCalled());
        expect(onAddServer.mock.calls[0][0]).toMatchObject({ migration_source: '/srv/old', migration_mode: 'move' });
    });
});
