/**
 * 권한 키는 봇 설정 저장으로 바뀌지 않는다(L61). 데몬이 `ignored`로 알린 키를 GUI도 버리지 않고 알린다 (CLI와 같게).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { setDiscordI18n, useDiscordStore } from '../stores/useDiscordStore';

describe('봇 설정 저장의 ignored', () => {
    afterEach(() => {
        delete window.showToast;
        setDiscordI18n((key: string, fallback?: string) => fallback || key);
    });

    it('브리지는 데몬이 알린 ignored를 그대로 넘긴다', async () => {
        const invoke = vi.fn().mockResolvedValue({
            status: 200,
            data: { success: true, message: 'Bot config saved', ignored: ['nodeSettings.1.permissions'] },
        });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } as never });
        const res = await api.botConfigSave({ prefix: '!x' });
        expect(res.success).toBe(true);
        expect(res.ignored).toEqual(['nodeSettings.1.permissions']);
    });

    describe('저장', () => {
        beforeEach(() => {
            useDiscordStore.getState()._resetForTest();
            useDiscordStore.setState({ _botConfigLoaded: true });
        });

        it('무시된 키가 있으면 저장했다고만 하지 않고 알린다', async () => {
            setDiscordI18n((key: string) => key);
            window.showToast = vi.fn();
            window.api = {
                botConfigLoad: vi.fn().mockResolvedValue({ cloud: {}, nodeSettings: { 1: { permissions: { x: 1 } } } }),
                botConfigSave: vi.fn().mockResolvedValue({ success: true, ignored: ['nodeSettings.1.permissions'] }),
            } as unknown as typeof window.api;
            useDiscordStore.setState({
                discordCloudRelayUrl: 'https://relay.example',
                nodeSettings: { 1: { permissions: { x: 2 } } },
            });
            expect(await useDiscordStore.getState().saveConfig()).toBe(true);
            expect(vi.mocked(window.showToast).mock.calls.at(-1)!.slice(0, 2)).toEqual([
                'discord.permission_keys_ignored',
                'warning',
            ]);
            // 데몬이 버린 값은 화면에 남기지 않는다
            expect((useDiscordStore.getState().nodeSettings as Record<number, { permissions: { x: number } }>)[1].permissions.x).toBe(1);
        });
    });
});
