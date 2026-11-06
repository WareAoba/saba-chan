import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerSettings } from '../hooks/useServerSettings';
import type { Server } from '../stores/useServerStore';
import type { ModuleInfo } from '../types/daemon';

const modules = [
    { name: 'game', settings: { fields: [{ name: 'title', field_type: 'text' }] } },
] as unknown as ModuleInfo[];

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
    } as unknown as typeof window.api;
});

// 사용자 요청(2026-10-29): 게임 채팅을 모두 보내거나 감지 단어로 시작하는 것만 보낸다 (데몬 game_chat·game_chat_keyword)
describe('게임 채팅 전달 칸', () => {
    it('저장된 값으로 채우고, 바꾼 칸만 보낸다', async () => {
        const { result } = setup();
        const server = {
            id: 's',
            name: 'S',
            module: 'game',
            game_chat: 'all',
            game_chat_keyword: '!d',
        } as unknown as Server;
        await act(async () => result.current.handleOpenSettings(server));
        expect(result.current.settingsValues).toMatchObject({ game_chat: 'all', game_chat_keyword: '!d' });
        await act(async () => result.current.handleSaveSettings());
        expect(vi.mocked(window.api.instanceUpdateSettings).mock.calls[0][1]).not.toHaveProperty('game_chat');

        act(() => result.current.handleSettingChange('game_chat', 'keyword'));
        act(() => result.current.handleSettingChange('game_chat_keyword', ' !디코 '));
        await act(async () => result.current.handleSaveSettings());
        expect(vi.mocked(window.api.instanceUpdateSettings).mock.calls[1][1]).toMatchObject({
            game_chat: 'keyword',
            game_chat_keyword: '!디코',
        });
    });

    it('예전 데몬처럼 값이 없으면 보내지 않기로 본다', async () => {
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings({ id: 's', name: 'S', module: 'game' } as Server));
        expect(result.current.settingsValues).toMatchObject({ game_chat: 'off', game_chat_keyword: '' });
        await act(async () => result.current.handleSaveSettings());
        expect(vi.mocked(window.api.instanceUpdateSettings).mock.calls[0][1]).not.toHaveProperty('game_chat');
        expect(vi.mocked(window.api.instanceUpdateSettings).mock.calls[0][1]).not.toHaveProperty('game_chat_keyword');
    });
});

// 사용자 지정(2026-11-06): 사바쨩 탭의 "조작 가능한 사람만 접속" (데몬 controllers_only)
describe('조작 가능한 사람만 접속 칸', () => {
    it('저장된 값으로 채우고, 바꿨을 때만 보낸다', async () => {
        const { result } = setup();
        const server = { id: 's', name: 'S', module: 'game', controllers: [{ name: 'Steve' }] } as unknown as Server;
        await act(async () => result.current.handleOpenSettings(server));
        expect(result.current.settingsValues).toMatchObject({ controllers_only: false });
        await act(async () => result.current.handleSaveSettings());
        expect(vi.mocked(window.api.instanceUpdateSettings).mock.calls[0][1]).not.toHaveProperty('controllers_only');

        act(() => result.current.handleSettingChange('controllers_only', true));
        await act(async () => result.current.handleSaveSettings());
        expect(vi.mocked(window.api.instanceUpdateSettings).mock.calls[1][1]).toMatchObject({ controllers_only: true });
    });
});
