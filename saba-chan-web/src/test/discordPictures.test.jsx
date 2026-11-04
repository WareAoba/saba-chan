import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeAll, describe, expect, it, vi } from 'vitest';
import { Avatar } from '../components/Avatar';
import { DiscordTab } from '../components/DiscordTab';
import { initI18n } from '../i18n';
import { useApp } from '../store';

const flush = () => act(async () => {});
const ICON = 'https://cdn.discordapp.com/icons/1/a.png';

describe('디스코드 서버·멤버 사진', () => {
    beforeAll(() => initI18n('en'));

    it('주소가 있으면 사진, 없거나 읽지 못하면 이름 첫 글자', () => {
        const { container, rerender } = render(<Avatar url={ICON} name="acorn" />);
        const img = container.querySelector('img');
        expect(img.getAttribute('src')).toBe(ICON);
        fireEvent.error(img);
        expect(container.querySelector('img')).toBeNull();
        expect(container.textContent).toBe('A');
        rerender(<Avatar url="javascript:alert(1)" name="dotori" />);
        expect(container.querySelector('img')).toBeNull();
        expect(container.textContent).toBe('D');
    });

    it('서버 고르기 칩에 디스코드 서버 아이콘을 보인다', async () => {
        useApp.setState((s) => ({ session: { ...s.session, hostId: null } }));
        const api = new Proxy({}, { get: (t, k) => t[k] ?? vi.fn().mockResolvedValue({}) });
        const relay = {
            guilds: vi.fn().mockResolvedValue({
                ok: true,
                data: [
                    { id: '1', name: 'Gather Craft', icon: ICON, hasPc: false, can: { manage: true } },
                    { id: '2', name: 'No Icon', icon: null, hasPc: false, can: { manage: true } },
                ],
            }),
            settings: vi.fn().mockResolvedValue({ ok: true, data: {} }),
            channels: vi.fn().mockResolvedValue({ ok: true, data: { botAvailable: true, channels: [], chatbot: [] } }),
            chatbot: vi.fn().mockResolvedValue({ ok: true, data: { settings: {}, hasApiKey: false } }),
        };
        render(<DiscordTab api={api} relay={relay} />);
        await flush();
        await flush();
        const tabs = screen.getAllByRole('tab');
        expect(tabs[0].querySelector('img')?.getAttribute('src')).toBe(ICON);
        expect(tabs[1].querySelector('img')).toBeNull();
        expect(tabs[1].textContent).toContain('N');
    });
});
