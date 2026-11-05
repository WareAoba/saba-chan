import { act, render, screen } from '@testing-library/react';
import { beforeAll, describe, expect, it, vi } from 'vitest';
import { DiscordTab } from '../components/DiscordTab';
import { initI18n } from '../i18n';
import { useApp } from '../store';
import { mockApi, mockRelay } from './mocks';

// 디스코드 서버·멤버 사진 (사용자 11-06): 이 PC 의 서버는 데몬이 준 주소를 쓴다
const G = '111111111111111111';
const G2 = '444444444444444444';
const U = '333333333333333333';
const DAEMON_ICON = 'https://cdn.discordapp.com/icons/111/daemon.png';
const RELAY_ICON = 'https://cdn.discordapp.com/icons/444/relay.png';
const AVATAR = 'https://cdn.discordapp.com/avatars/333/a.png';
const flush = () => act(async () => {});

function setup() {
    useApp.setState((s) => ({ session: { ...s.session, hostId: 'h1' } }));
    const api = new Proxy(
        {
            relayListHostNodes: vi
                .fn()
                .mockResolvedValue([{ guildId: G, guildName: 'Gather Craft', iconUrl: DAEMON_ICON }]),
            relayListNodeMembers: vi
                .fn()
                .mockResolvedValue({
                    members: [{ id: U, username: 'acorn', displayName: 'Acorn', avatarUrl: AVATAR }],
                    partial: false,
                }),
            discordSettingsGet: vi.fn().mockResolvedValue({ guildId: G, locale: 'auto', music: {}, tts: {} }),
            chatbotGuildGet: vi.fn().mockResolvedValue({ success: true, settings: {}, hasApiKey: false }),
        },
        { get: (t, k) => t[k as keyof typeof t] ?? vi.fn().mockResolvedValue({}) },
    );
    const relay = {
        guilds: vi.fn().mockResolvedValue({
            ok: true,
            data: [
                { id: G, name: 'Gather Craft', icon: null, hasPc: true, can: { manage: true, members: true } },
                { id: G2, name: 'Friends', icon: RELAY_ICON, hasPc: false, can: { manage: true } },
            ],
        }),
        channels: vi.fn().mockResolvedValue({ ok: true, data: { botAvailable: true, channels: [], chatbot: [] } }),
        moderation: vi.fn().mockResolvedValue({
            ok: true,
            data: {
                botAvailable: true,
                can: { moderateMembers: true },
                channels: [],
                mutes: [],
                sanctions: [{ userId: U, name: 'Acorn', kind: 'chat', until: null, inGuild: true }],
            },
        }),
        modMembers: vi.fn().mockResolvedValue({ ok: true, data: [] }),
    };
    const view = render(<DiscordTab api={mockApi(api)} relay={mockRelay(relay)} />);
    return { api, relay, view };
}

describe('디스코드 서버·멤버 사진', () => {
    beforeAll(() => initI18n('en'));

    it('서버 고르기 칩에 서버 사진을 보인다 — 이 PC 의 서버는 데몬이 준 주소', async () => {
        const { view } = setup();
        await flush();
        await flush();
        const srcs = [...view.container.querySelectorAll('.guild-chips img')].map((i) => i.getAttribute('src'));
        expect(srcs).toEqual([DAEMON_ICON, RELAY_ICON]);
    });

    it('제재 중인 사람 줄에 프로필 사진을 보인다 — 이 PC 의 서버는 멤버를 데몬에서 받는다', async () => {
        const { api, relay, view } = setup();
        await flush();
        await flush();
        await flush();
        expect(screen.getByText('Acorn')).toBeTruthy();
        const row = screen.getByText('Acorn').closest('li') as HTMLElement;
        expect(row.querySelector('img')?.getAttribute('src')).toBe(AVATAR);
        expect(api.relayListNodeMembers).toHaveBeenCalledWith(G, '');
        expect(relay.modMembers).not.toHaveBeenCalled();
        view.unmount();
    });
});
