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
        fireEvent.click(screen.getByTitle('Browse'));
        expect(await screen.findByText(/Minecraft Java/)).toBeTruthy();
        expect(window.api.migrationScanDir).toHaveBeenCalledWith('/srv/old');
        expect(window.api.moduleDetect).not.toHaveBeenCalled();
    });
});
