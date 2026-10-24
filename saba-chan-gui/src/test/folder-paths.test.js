import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { FolderPathsSection } from '../components/Modals/FolderPathsSection';

const paths = {
    ok: true,
    data_dir: 'C:/Users/me/AppData/Roaming/saba-chan',
    install_dir: 'C:/Program Files/Saba-chan',
    default_install_dir: 'C:/Program Files/Saba-chan',
    instances_dir: 'C:/Users/me/AppData/Roaming/saba-chan/instances',
    modules: {
        path: 'D:/Games/Saba/modules',
        default: 'C:/Users/me/AppData/Roaming/saba-chan/modules',
        source: 'config',
    },
    extensions: {
        path: 'C:/Users/me/AppData/Roaming/saba-chan/extensions',
        default: 'C:/Users/me/AppData/Roaming/saba-chan/extensions',
        source: 'default',
    },
};

describe('설정 > 고급: 폴더 위치 (/api/system/paths)', () => {
    beforeEach(() => {
        window.api = {
            systemPaths: vi.fn().mockResolvedValue(paths),
            systemPathsMove: vi.fn(),
            openFolderDialog: vi.fn().mockResolvedValue('E:/Saba'),
        };
    });

    it('지금 폴더와 출처, 기본값을 보인다', async () => {
        await act(async () => render(<FolderPathsSection />));
        expect(screen.getByText('D:/Games/Saba/modules')).toBeInTheDocument();
        expect(screen.getByText(/C:\/Users\/me\/AppData\/Roaming\/saba-chan\/modules/)).toBeInTheDocument();
        expect(screen.getByText('C:/Users/me/AppData/Roaming/saba-chan/instances')).toBeInTheDocument();
    });

    it('폴더를 고르고 확인해야 옮기고, 옮긴 뒤 다시 읽는다', async () => {
        window.api.systemPathsMove.mockResolvedValue({ ok: true, path: 'E:/Saba/extensions', left_behind: [] });
        await act(async () => render(<FolderPathsSection />));
        const [, extMove] = screen.getAllByRole('button', { name: /Move…/ });
        await act(async () => fireEvent.click(extMove));
        expect(window.api.systemPathsMove).not.toHaveBeenCalled();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /^\s*Move$/ })));
        expect(window.api.systemPathsMove).toHaveBeenCalledWith('extensions', 'E:/Saba');
        expect(screen.getByText(/Moved to E:\/Saba\/extensions/)).toBeInTheDocument();
        expect(window.api.systemPaths).toHaveBeenCalledTimes(2);
    });

    it('데몬이 거절하면 이유를 번역해 보이고 다시 읽지 않는다', async () => {
        window.api.systemPathsMove.mockResolvedValue({ ok: false, error_code: 'paths_servers_running', error: 'survival' });
        await act(async () => render(<FolderPathsSection />));
        await act(async () => fireEvent.click(screen.getAllByRole('button', { name: /Move…/ })[0]));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /^\s*Move$/ })));
        expect(screen.getByRole('alert')).toHaveTextContent('Stop these servers first: survival');
        expect(window.api.systemPaths).toHaveBeenCalledTimes(1);
    });
});
