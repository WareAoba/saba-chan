import { act, render, screen } from '@testing-library/react';
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

describe('설정 > 저장 공간: 폴더 위치 (/api/system/paths)', () => {
    beforeEach(() => {
        window.api = {
            systemPaths: vi.fn().mockResolvedValue(paths),
        } as unknown as typeof window.api;
    });

    it('지금 폴더와 출처, 기본값을 보인다', async () => {
        await act(async () => render(<FolderPathsSection />));
        expect(screen.getByText('D:/Games/Saba/modules')).toBeInTheDocument();
        expect(screen.getByText(/C:\/Users\/me\/AppData\/Roaming\/saba-chan\/modules/)).toBeInTheDocument();
        expect(screen.getByText('C:/Users/me/AppData/Roaming/saba-chan/instances')).toBeInTheDocument();
    });
});
