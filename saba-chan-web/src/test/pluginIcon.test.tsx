import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeAll, describe, expect, it, vi } from 'vitest';
import { ModulesTab } from '../components/ModulesTab';
import { initI18n } from '../i18n';
import { mockApi } from './mocks';

const flush = () => act(async () => {});
const PNG = 'data:image/png;base64,iVBORw0KGgo=';

describe('플러그인 사진', () => {
    beforeAll(() => initI18n('en'));

    it('아이콘이 있는 플러그인은 데몬이 준 그림을, 없으면 이름 첫 글자를 보인다', async () => {
        const api = new Proxy(
            {
                extensionList: vi.fn().mockResolvedValue({
                    extensions: [
                        { id: 'steamcmd', name: 'SteamCMD', has_icon: true, enabled: true },
                        { id: 'plain', name: 'plain', has_icon: false, enabled: false },
                    ],
                }),
                extensionFetchManifest: vi.fn().mockResolvedValue({ extensions: [] }),
                extensionCheckUpdates: vi.fn().mockResolvedValue({ updates: [] }),
                extensionIcon: vi.fn().mockResolvedValue(PNG),
                moduleManifest: vi.fn().mockResolvedValue({}),
            },
            { get: (t, k) => t[k as keyof typeof t] ?? vi.fn().mockResolvedValue({}) },
        );
        render(<ModulesTab api={mockApi(api)} />);
        fireEvent.click(screen.getByRole('tab', { name: 'Extensions' }));
        await flush();
        await flush();
        expect(api.extensionIcon).toHaveBeenCalledTimes(1);
        expect(api.extensionIcon).toHaveBeenCalledWith('steamcmd');
        const rows = screen.getAllByRole('listitem');
        expect(rows[0].querySelector('img')?.getAttribute('src')).toBe(PNG);
        expect(rows[1].querySelector('img')).toBeNull();
        expect(rows[1].querySelector('.server-icon')?.textContent).toBe('P');
    });

    it('설치된 플러그인 목록을 못 읽으면 "없음"이 아니라 읽기 실패와 다시 시도를 보인다', async () => {
        const extensionList = vi
            .fn()
            .mockResolvedValueOnce({ error: 'not connected', error_code: 'ECONNREFUSED' })
            .mockResolvedValue({ extensions: [{ id: 'plain', name: 'plain', has_icon: false, enabled: true }] });
        const api = new Proxy(
            {
                extensionList,
                extensionFetchManifest: vi.fn().mockResolvedValue({ extensions: [] }),
                extensionCheckUpdates: vi.fn().mockResolvedValue({ updates: [] }),
                moduleManifest: vi.fn().mockResolvedValue({}),
            },
            { get: (t, k) => t[k as keyof typeof t] ?? vi.fn().mockResolvedValue({}) },
        );
        render(<ModulesTab api={mockApi(api)} />);
        fireEvent.click(screen.getByRole('tab', { name: 'Extensions' }));
        await flush();
        await flush();
        expect(screen.queryByText('No extensions installed')).toBeNull();
        expect(screen.getByText('Failed to load the extension list.')).toBeTruthy();
        fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
        await flush();
        await flush();
        expect(extensionList).toHaveBeenCalledTimes(2);
        expect(screen.getByText('plain')).toBeTruthy();
    });
});
