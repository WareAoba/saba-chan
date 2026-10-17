import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DataTransferSection } from '../components/Modals/DataTransferSection';

const preview = (overwrite) => ({
    success: true,
    dryRun: true,
    appVersion: '0.1.3',
    secrets: false,
    instances: {
        added: [{ id: 'a', name: 'Alpha' }],
        replaced: overwrite ? [{ id: 'b', name: 'Beta' }] : [],
        skipped: overwrite ? [] : [{ id: 'b', name: 'Beta', reason: 'exists' }],
    },
    missingModules: ['zomboid'],
    config: ['gui'],
    nodeToken: null,
    path: '/tmp/x.zip',
});

describe('DataTransferSection', () => {
    afterEach(() => {
        delete window.api.dataExport;
        delete window.api.dataImport;
    });

    it('미리보기 뒤 같은 파일로 가져오고, 덮어쓰기를 바꾸면 미리보기를 다시 받는다', async () => {
        window.api.dataExport = vi.fn();
        window.api.dataImport = vi.fn(async ({ dryRun, overwrite }) =>
            dryRun ? preview(overwrite) : { ...preview(overwrite), dryRun: false },
        );
        render(<DataTransferSection />);
        fireEvent.click(screen.getByRole('button', { name: /Choose file to import/ }));
        expect(await screen.findByText(/Skip Beta \(a server with this ID already exists\)/)).toBeTruthy();
        expect(screen.getByText(/Some modules aren't on this PC: zomboid/)).toBeTruthy();
        expect(window.api.dataImport).toHaveBeenLastCalledWith({ path: undefined, dryRun: true, overwrite: false });

        fireEvent.click(screen.getByLabelText('Overwrite existing servers'));
        expect(await screen.findByText(/Overwrite: Beta/)).toBeTruthy();
        expect(window.api.dataImport).toHaveBeenLastCalledWith({ path: '/tmp/x.zip', dryRun: true, overwrite: true });

        fireEvent.click(screen.getByRole('button', { name: /^Import$/ }));
        await waitFor(() =>
            expect(window.api.dataImport).toHaveBeenLastCalledWith({
                path: '/tmp/x.zip',
                dryRun: false,
                overwrite: true,
            }),
        );
        expect(await screen.findByText('Imported')).toBeTruthy();
    });

    it('취소하면 아무것도 보이지 않고, 데몬 오류는 코드별 문구로 보인다', async () => {
        window.api.dataExport = vi
            .fn()
            .mockResolvedValueOnce({ canceled: true })
            .mockResolvedValueOnce({ success: false, error: 'boom', error_code: 'WRITE_FAILED' });
        window.api.dataImport = vi.fn();
        render(<DataTransferSection />);
        const btn = screen.getByRole('button', { name: /^Export$/ });
        fireEvent.click(btn);
        await waitFor(() => expect(window.api.dataExport).toHaveBeenCalledWith({ secrets: false }));
        expect(screen.queryByRole('alert')).toBeNull();
        fireEvent.click(btn);
        expect(await screen.findByText(/Couldn't write the file/)).toBeTruthy();
    });
});
