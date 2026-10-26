/**
 * 원격 데몬 쪽 경로 고르기 창 (D2): 원격 데몬을 다루는 중 대화상자 대신 그 PC의 폴더를 보여 준다.
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { pickRemotePath, setRemotePathPicker } from '../api/remotePathPicker';
import RemotePathPicker from '../components/shell/RemotePathPicker';

const listing = {
    '': {
        ok: true,
        path: null,
        parent: null,
        home: '/home/me',
        sep: '/',
        entries: [{ name: '/', path: '/', dir: true }],
    },
    '/home/me': {
        ok: true,
        path: '/home/me',
        parent: '/home',
        home: '/home/me',
        sep: '/',
        entries: [
            { name: 'worlds', path: '/home/me/worlds', dir: true },
            { name: 'backup.zip', path: '/home/me/backup.zip', dir: false },
        ],
    },
    '/home/me/worlds': {
        ok: true,
        path: '/home/me/worlds',
        parent: '/home/me',
        home: '/home/me',
        sep: '/',
        entries: [],
    },
    '/home': { ok: false, error_code: 'browse_denied' },
};

describe('원격 경로 고르기 창', () => {
    beforeEach(() => {
        window.api = { systemBrowse: vi.fn(async (path) => listing[path || '']) };
    });
    afterEach(() => setRemotePathPicker(null));

    it('그 PC의 홈에서 시작하고, 폴더를 열어 지금 폴더를 고른다', async () => {
        await act(async () => render(<RemotePathPicker />));
        let result;
        await act(async () => {
            result = pickRemotePath({ kind: 'folder', extensions: null, defaultName: null });
        });
        expect(window.api.systemBrowse).toHaveBeenLastCalledWith('/home/me', null);
        expect(screen.getByRole('dialog', { name: 'Choose a folder on the remote PC' })).toBeInTheDocument();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'worlds' })));
        expect(screen.getByDisplayValue('/home/me/worlds')).toBeInTheDocument();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Choose this folder' })));
        await expect(result).resolves.toBe('/home/me/worlds');
    });

    it('파일은 고른 확장자만 묻고, 고른 파일을 돌려준다. 취소는 null', async () => {
        await act(async () => render(<RemotePathPicker />));
        let result;
        await act(async () => {
            result = pickRemotePath({ kind: 'file', extensions: ['zip'], defaultName: null });
        });
        expect(window.api.systemBrowse).toHaveBeenLastCalledWith('/home/me', ['zip']);
        const choose = screen.getByRole('button', { name: 'Choose' });
        expect(choose).toBeDisabled();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'backup.zip' })));
        await act(async () => fireEvent.click(choose));
        await expect(result).resolves.toBe('/home/me/backup.zip');

        await act(async () => {
            result = pickRemotePath({ kind: 'file', extensions: null, defaultName: null });
        });
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Cancel' })));
        await expect(result).resolves.toBeNull();
    });

    it('저장은 지금 폴더에 적은 이름을 붙이고, 열 수 없는 폴더는 이유를 보인다', async () => {
        await act(async () => render(<RemotePathPicker />));
        let result;
        await act(async () => {
            result = pickRemotePath({ kind: 'save', extensions: ['zip'], defaultName: 'export.zip' });
        });
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Up' })));
        expect(screen.getByRole('alert')).toHaveTextContent("You can't open that folder");
        const name = screen.getByRole('textbox', { name: 'File name' });
        expect(name).toHaveValue('export.zip');
        fireEvent.change(name, { target: { value: 'mine' } });
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Save here' })));
        await expect(result).resolves.toBe('/home/me/mine.zip');
    });
});
