import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerSettings } from '../hooks/useServerSettings';

const modules = [{ name: 'game', settings: { fields: [{ name: 'title', field_type: 'text' }] } }];

const setup = () =>
    renderHook(() =>
        useServerSettings({ servers: [], modules, setModal: vi.fn(), setProgressBar: vi.fn(), fetchServers: vi.fn() }),
    );

beforeEach(() => {
    window.showToast = vi.fn();
    window.api = {
        serverList: vi.fn().mockResolvedValue(null),
        instanceReadLiveSettings: vi.fn().mockResolvedValue({ success: true, settings: { title: 'x' } }),
        instanceUpdateSettings: vi.fn().mockResolvedValue({ success: true }),
    };
});

describe('메모리 상한 칸', () => {
    it('저장된 값으로 채우고, 바꾸지 않으면 보내지 않는다', async () => {
        const { result } = setup();
        await act(async () =>
            result.current.handleOpenSettings({ id: 's', name: 'S', module: 'game', memory_limit_mb: 4096 }),
        );
        expect(result.current.settingsValues.memory_limit_mb).toBe('4096');
        await act(async () => result.current.handleSaveSettings());
        expect(window.api.instanceUpdateSettings.mock.calls[0][1]).not.toHaveProperty('memory_limit_mb');
    });

    it('바꾸면 그 값을, 비우면 null(제한 없음)을 보낸다', async () => {
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings({ id: 's', name: 'S', module: 'game' }));
        expect(result.current.settingsValues.memory_limit_mb).toBe('');
        act(() => result.current.handleSettingChange('memory_limit_mb', ' 6144 '));
        await act(async () => result.current.handleSaveSettings());
        expect(window.api.instanceUpdateSettings.mock.calls[0][1]).toMatchObject({ memory_limit_mb: '6144' });

        await act(async () =>
            result.current.handleOpenSettings({ id: 's', name: 'S', module: 'game', memory_limit_mb: 2048 }),
        );
        act(() => result.current.handleSettingChange('memory_limit_mb', ''));
        await act(async () => result.current.handleSaveSettings());
        expect(window.api.instanceUpdateSettings.mock.calls[1][1]).toMatchObject({ memory_limit_mb: null });
    });

    it('형식이 틀리면 "잘못된 형식입니다" 토스트만 띄운다', async () => {
        window.api.instanceUpdateSettings = vi.fn().mockResolvedValue({
            error: 'invalid format',
            error_code: 'invalid_format',
            fields: [{ field: 'memory_limit_mb' }],
        });
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings({ id: 's', name: 'S', module: 'game' }));
        act(() => result.current.handleSettingChange('memory_limit_mb', '4G'));
        await act(async () => result.current.handleSaveSettings());
        expect(window.showToast).toHaveBeenCalledWith(expect.stringMatching(/format/i), 'warning', 4000, undefined);
    });

    it('자동 재시작 토글을 끄면 글자로 바뀌어도 false로 저장한다', async () => {
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings({ id: 's', name: 'S', module: 'game' }));
        act(() => result.current.handleSettingChange('auto_restart', false));
        await act(async () => result.current.handleSaveSettings());
        expect(window.api.instanceUpdateSettings.mock.calls[0][1]).toMatchObject({ auto_restart: false });
    });
});
