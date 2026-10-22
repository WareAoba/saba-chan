import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerSettings } from '../hooks/useServerSettings';

// 창이 열린 동안 CLI·디스코드·익스텐션이 바꾼 값을, 손대지 않은 칸이 연 때의 값으로 되돌리지 않는다.
// 인스턴스 설정 저장도 앱 설정(#509)처럼 바뀐 것만 보낸다
const modules = [
    {
        name: 'pal',
        settings: {
            fields: [
                { name: 'description', field_type: 'text' },
                { name: 'max_players', field_type: 'number' },
                { name: 'pvp', field_type: 'boolean' },
            ],
        },
    },
];
const srv = {
    id: 'a',
    name: 'A',
    module: 'pal',
    status: 'stopped',
    module_settings: { description: '옛 설명', max_players: 8, pvp: false, auto_restart: true },
    extension_data: { cpu_limit: 2, note: 'x' },
};

const open = async () => {
    const { result } = renderHook(() =>
        useServerSettings({
            servers: [srv],
            modules,
            setModal: vi.fn(),
            setProgressBar: vi.fn(),
            fetchServers: vi.fn(),
        }),
    );
    await act(async () => result.current.handleOpenSettings(srv));
    return result;
};

describe('서버 설정 저장은 바뀐 것만 보낸다', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = {
            serverList: vi.fn().mockResolvedValue(null),
            instanceReadLiveSettings: vi
                .fn()
                .mockResolvedValue({ success: true, settings: { description: '옛 설명', max_players: 8, pvp: false } }),
            instanceUpdateSettings: vi.fn().mockResolvedValue({ success: true }),
        };
    });

    it('고친 칸만 싣고, 손대지 않은 칸·재시작 표시·익스텐션 값은 싣지 않는다', async () => {
        const result = await open();
        act(() => result.current.handleSettingChange('description', '새 설명'));
        await act(async () => result.current.handleSaveSettings());
        const sent = window.api.instanceUpdateSettings.mock.calls[0][1];
        expect(sent.description).toBe('새 설명');
        expect(sent).not.toHaveProperty('max_players');
        expect(sent).not.toHaveProperty('pvp');
        expect(sent).not.toHaveProperty('auto_restart');
        expect(sent).not.toHaveProperty('restart_on_unknown_exit');
        expect(sent).not.toHaveProperty('extension_data');
    });

    it('실제 설정이 저장값과 다르면 그 칸은 맞추려고 싣는다', async () => {
        window.api.instanceReadLiveSettings.mockResolvedValue({
            success: true,
            settings: { description: '옛 설명', max_players: 16, pvp: false },
        });
        const result = await open();
        await act(async () => result.current.handleSaveSettings());
        const sent = window.api.instanceUpdateSettings.mock.calls[0][1];
        expect(sent.max_players).toBe(16);
        expect(sent).not.toHaveProperty('description');
    });

    it('바꾼 재시작 표시는 그것만 싣는다', async () => {
        const result = await open();
        act(() => result.current.handleSettingChange('auto_restart', false));
        await act(async () => result.current.handleSaveSettings());
        const sent = window.api.instanceUpdateSettings.mock.calls[0][1];
        expect(sent.auto_restart).toBe(false);
        expect(sent).not.toHaveProperty('restart_on_unknown_exit');
        expect(sent).not.toHaveProperty('extension_data');
    });
});
