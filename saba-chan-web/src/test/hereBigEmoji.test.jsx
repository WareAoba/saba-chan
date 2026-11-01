import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeAll, describe, expect, it, vi } from 'vitest';
import { DiscordTab } from '../components/DiscordTab';
import { initI18n } from '../i18n';
import { useApp } from '../store';

// 이 PC 에 붙은 서버에서도 큰 이모지(원본은 릴레이)는 웹에서 바로 바꾼다
const G = '111111111111111111';
const flush = () => act(async () => {});

describe('이 PC 의 서버 — 큰 이모지', () => {
    beforeAll(() => initI18n('en'));

    it('관리 권한이 있으면 릴레이 값으로 토글을 보이고, 바꾸면 bigEmoji 만 보낸다', async () => {
        useApp.setState((s) => ({ session: { ...s.session, hostId: 'h1' } }));
        const api = new Proxy(
            { relayListHostNodes: vi.fn().mockResolvedValue([{ guildId: G, guildName: 'Gather Craft' }]) },
            { get: (t, k) => t[k] ?? vi.fn().mockResolvedValue({}) },
        );
        const relay = {
            guilds: vi.fn().mockResolvedValue({ ok: true, data: [{ id: G, name: 'Gather Craft', hasPc: true, can: { manage: true } }] }),
            settings: vi.fn().mockResolvedValue({ ok: true, data: { bigEmoji: false } }),
            saveSettings: vi.fn().mockResolvedValue({ ok: true, data: { bigEmoji: true } }),
        };
        render(<DiscordTab api={api} relay={relay} />);
        await flush();
        await flush();
        const toggle = screen.getByRole('checkbox', { name: /Big emoji/ });
        expect(toggle.checked).toBe(false);
        fireEvent.click(toggle);
        await flush();
        expect(relay.saveSettings).toHaveBeenCalledWith(G, { bigEmoji: true });
        expect(screen.getByRole('checkbox', { name: /Big emoji/ }).checked).toBe(true);
    });
});
