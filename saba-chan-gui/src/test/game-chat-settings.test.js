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

// 사용자 요청(2026-10-29): 게임 채팅을 모두 보내거나 감지 단어로 시작하는 것만 보낸다 (데몬 game_chat·game_chat_keyword)
describe('게임 채팅 전달 칸', () => {
    it('저장된 값으로 채우고, 바꾼 칸만 보낸다', async () => {
        const { result } = setup();
        const server = { id: 's', name: 'S', module: 'game', game_chat: 'all', game_chat_keyword: '!d' };
        await act(async () => result.current.handleOpenSettings(server));
        expect(result.current.settingsValues).toMatchObject({ game_chat: 'all', game_chat_keyword: '!d' });
        await act(async () => result.current.handleSaveSettings());
        expect(window.api.instanceUpdateSettings.mock.calls[0][1]).not.toHaveProperty('game_chat');

        act(() => result.current.handleSettingChange('game_chat', 'keyword'));
        act(() => result.current.handleSettingChange('game_chat_keyword', ' !디코 '));
        await act(async () => result.current.handleSaveSettings());
        expect(window.api.instanceUpdateSettings.mock.calls[1][1]).toMatchObject({
            game_chat: 'keyword',
            game_chat_keyword: '!디코',
        });
    });

    it('예전 데몬처럼 값이 없으면 보내지 않기로 본다', async () => {
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings({ id: 's', name: 'S', module: 'game' }));
        expect(result.current.settingsValues).toMatchObject({ game_chat: 'off', game_chat_keyword: '' });
        await act(async () => result.current.handleSaveSettings());
        expect(window.api.instanceUpdateSettings.mock.calls[0][1]).not.toHaveProperty('game_chat');
        expect(window.api.instanceUpdateSettings.mock.calls[0][1]).not.toHaveProperty('game_chat_keyword');
    });
});
