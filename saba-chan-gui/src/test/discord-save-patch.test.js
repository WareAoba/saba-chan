import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useDiscordStore } from '../stores/useDiscordStore';

beforeEach(() => {
    useDiscordStore.getState()._resetForTest();
});

describe('봇 설정은 바뀐 칸만 보낸다 (0.1.10 결정 1)', () => {
    it('한 길드의 한 칸을 바꾸면 그 길드의 그 칸만 싣는다', async () => {
        const daemon = {
            prefixes: ['사바쨩'],
            cloud: { relayUrl: 'https://r', hostId: 'h1' },
            nodeSettings: { g1: { memo: 'a', members: { u: 2 } }, g2: { memo: 'b' } },
        };
        window.api = {
            botConfigLoad: vi.fn(async () => daemon),
            botConfigSave: vi.fn(async () => ({ success: true })),
        };
        await useDiscordStore.getState().loadConfig();
        const nodes = useDiscordStore.getState().nodeSettings;
        useDiscordStore.setState({ nodeSettings: { ...nodes, g1: { ...nodes.g1, memo: 'c' } }, discordCloudHostId: 'h2' });
        expect(await useDiscordStore.getState().saveConfig()).toBe(true);
        expect(window.api.botConfigSave).toHaveBeenCalledWith({
            nodeSettings: { g1: { memo: 'c' } },
            cloud: { hostId: 'h2' },
        });
    });
});
