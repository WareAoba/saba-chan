import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { beforeAll, describe, expect, it, vi } from 'vitest';
import { DiscordTab } from '../components/DiscordTab';
import { initI18n } from '../i18n';
import { useApp } from '../store';

// 이 PC 에 붙은 서버 (사용자 결정 2026-11-03): 데몬이 원본인 값은 데몬 경로, 배정·기억은 릴레이
const G = '111111111111111111';
const flush = () => act(async () => {});
const GUARD = 'Block messages while chat link is off';
const SETTINGS = {
    guildId: G,
    locale: 'auto',
    sleeping: false,
    sleepBlocks: ['commands', 'llm', 'music', 'tts', 'extras'],
    bedtime: null,
    bigEmoji: false,
    gameChatGuard: false,
    music: { enabled: true, ui: true },
    tts: { volume: 100, speed: 100, pitch: 0, names: 'multi' },
};

function setup(can = { manage: true }) {
    useApp.setState((s) => ({ session: { ...s.session, hostId: 'h1' } }));
    const api = new Proxy(
        {
            relayListHostNodes: vi.fn().mockResolvedValue([{ guildId: G, guildName: 'Gather Craft' }]),
            discordSettingsGet: vi.fn().mockResolvedValue(SETTINGS),
            discordSettingsSet: vi.fn(async (_g, patch) => ({ success: true, settings: { ...SETTINGS, ...patch } })),
            chatbotGuildGet: vi
                .fn()
                .mockResolvedValue({ success: true, settings: { enabled: false }, hasApiKey: false }),
            chatbotGuildSave: vi.fn(async (_g, patch) => ({ success: true, settings: patch, hasApiKey: false })),
            chatbotSave: vi.fn().mockResolvedValue({ success: true, hasApiKey: true }),
        },
        { get: (t, k) => t[k] ?? vi.fn().mockResolvedValue({}) },
    );
    const relay = {
        guilds: vi.fn().mockResolvedValue({
            ok: true,
            data: [{ id: G, name: 'Gather Craft', hasPc: true, can }],
        }),
        channels: vi.fn().mockResolvedValue({ ok: true, data: { botAvailable: true, channels: [], chatbot: [] } }),
        saveSettings: vi.fn(),
        saveChatbot: vi.fn(),
        moderation: vi
            .fn()
            .mockResolvedValue({ ok: true, data: { botAvailable: true, channels: [], mutes: [], sanctions: [] } }),
    };
    render(<DiscordTab api={api} relay={relay} />);
    return { api, relay };
}

describe('이 PC 의 디스코드 서버 설정', () => {
    beforeAll(() => initI18n('en'));

    it('큰 이모지와 코코낸내는 데몬 설정으로 바꾼다 (릴레이에 직접 쓰지 않는다)', async () => {
        const { api, relay } = setup();
        await flush();
        await flush();
        fireEvent.click(screen.getByRole('checkbox', { name: /^Big emoji/ }));
        await flush();
        expect(api.discordSettingsSet).toHaveBeenCalledWith(G, { bigEmoji: true });
        fireEvent.click(screen.getByRole('checkbox', { name: /Singing Saba-chan/ }));
        await flush();
        expect(api.discordSettingsSet).toHaveBeenLastCalledWith(G, {
            sleepBlocks: ['commands', 'llm', 'tts', 'extras'],
        });
        fireEvent.click(screen.getByRole('checkbox', { name: new RegExp(`^${GUARD}`) }));
        await flush();
        expect(api.discordSettingsSet).toHaveBeenLastCalledWith(G, { gameChatGuard: true });
        expect(relay.saveSettings).not.toHaveBeenCalled();
    });

    it('말하는 사바쨩 켜기는 서버별 설정, API 키는 이 PC 의 키로 넣는다', async () => {
        const { api } = setup();
        await flush();
        await flush();
        const talking = within(screen.getByRole('heading', { name: 'Talking Saba-chan' }).closest('section'));
        fireEvent.click(talking.getByRole('checkbox', { name: /On/ }));
        await flush();
        expect(api.chatbotGuildSave).toHaveBeenCalledWith(G, { enabled: true });
        fireEvent.click(talking.getByRole('button', { name: 'Add' }));
        fireEvent.change(screen.getByLabelText(/API key/, { selector: 'input' }), {
            target: { value: 'sk-123456789' },
        });
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await flush();
        await flush();
        expect(api.chatbotSave).toHaveBeenCalledWith({ apiKey: 'sk-123456789' });
    });

    it('이 PC 의 서버에도 서버 관리를 보인다 (릴레이 경로)', async () => {
        const { relay } = setup({ manage: false, channels: true, messages: true, members: true });
        await flush();
        await flush();
        expect(relay.moderation).toHaveBeenCalledWith(G);
        expect(screen.getByRole('heading', { name: 'Server management' })).toBeTruthy();
    });
});
