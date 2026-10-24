import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerSettings } from '../hooks/useServerSettings';

// 창을 연 뒤 다른 곳(CLI·디스코드·다른 창)이 같은 서버 설정을 바꿨으면 덮어쓰지 않는다.
// 연 때의 revision을 함께 보내고, 데몬이 settings_changed로 거절하면 알리고 목록을 다시 읽는다
const modules = [{ name: 'pal', settings: { fields: [{ name: 'description', field_type: 'text' }] } }];
const srv = {
    id: 'a',
    name: 'A',
    module: 'pal',
    status: 'stopped',
    revision: 'rev-open',
    module_settings: { description: '옛 설명' },
};

const open = async (setModal, fetchServers) => {
    const { result } = renderHook(() =>
        useServerSettings({ servers: [srv], modules, setModal, setProgressBar: vi.fn(), fetchServers }),
    );
    await act(async () => result.current.handleOpenSettings(srv));
    return result;
};

describe('서버 설정 저장은 연 때의 revision을 확인한다', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = {
            serverList: vi.fn().mockResolvedValue(null),
            instanceReadLiveSettings: vi.fn().mockResolvedValue({ success: true, settings: {} }),
            instanceUpdateSettings: vi.fn().mockResolvedValue({ success: true }),
        };
    });

    it('연 때의 revision을 함께 보낸다', async () => {
        const result = await open(vi.fn(), vi.fn());
        act(() => result.current.handleSettingChange('description', '새 설명'));
        await act(async () => result.current.handleSaveSettings());
        expect(window.api.instanceUpdateSettings.mock.calls[0][1].revision).toBe('rev-open');
    });

    it('열 때 실제 값 읽기가 캐시를 맞췄으면 그 뒤의 revision으로 저장한다', async () => {
        window.api.instanceReadLiveSettings.mockResolvedValue({ success: true, settings: {}, revision: 'rev-live' });
        const result = await open(vi.fn(), vi.fn());
        act(() => result.current.handleSettingChange('description', '새 설명'));
        await act(async () => result.current.handleSaveSettings());
        expect(window.api.instanceUpdateSettings.mock.calls[0][1].revision).toBe('rev-live');
    });

    it('settings_changed면 다른 곳에서 바뀌었다고 알리고 서버 목록을 다시 읽는다', async () => {
        window.api.instanceUpdateSettings.mockResolvedValue({
            error: 'settings changed',
            error_code: 'settings_changed',
            revision: 'rev-new',
        });
        const setModal = vi.fn();
        const fetchServers = vi.fn();
        const result = await open(setModal, fetchServers);
        act(() => result.current.handleSettingChange('description', '새 설명'));
        await act(async () => result.current.handleSaveSettings());
        const modal = setModal.mock.calls.at(-1)[0];
        expect(modal.type).toBe('failure');
        expect(modal.message).toMatch(/server_settings.settings_changed|changed elsewhere/);
        expect(fetchServers).toHaveBeenCalled();
    });
});
