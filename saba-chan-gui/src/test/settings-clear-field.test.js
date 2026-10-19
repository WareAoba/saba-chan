import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerSettings } from '../hooks/useServerSettings';

const modules = [
    {
        name: 'pal',
        settings: {
            fields: [
                { name: 'description', field_type: 'text' },
                { name: 'server_password', field_type: 'password' },
                { name: 'max_players', field_type: 'number' },
            ],
        },
    },
];
const srv = {
    id: 'a',
    name: 'A',
    module: 'pal',
    status: 'stopped',
    module_settings: { description: '옛 설명', server_password: 'old', max_players: 8 },
};

describe('선택 문자열 칸을 비우고 저장할 때', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = {
            serverList: vi.fn().mockResolvedValue(null),
            instanceUpdateSettings: vi.fn().mockResolvedValue({ success: true }),
        };
    });

    it('지운 설명·비밀번호는 빈 값으로 보낸다', async () => {
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
        act(() => {
            result.current.handleSettingChange('description', '');
            result.current.handleSettingChange('server_password', '');
        });
        await act(async () => result.current.handleSaveSettings());
        const sent = window.api.instanceUpdateSettings.mock.calls[0][1];
        expect(sent.description).toBe('');
        expect(sent.server_password).toBe('');
    });

    it('숫자 칸을 비우면 보내지 않는다 (빈 숫자는 데몬이 거절한다)', async () => {
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
        act(() => result.current.handleSettingChange('max_players', ''));
        await act(async () => result.current.handleSaveSettings());
        expect(window.api.instanceUpdateSettings.mock.calls[0][1]).not.toHaveProperty('max_players');
    });
});
