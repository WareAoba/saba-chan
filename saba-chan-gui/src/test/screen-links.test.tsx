/**
 * #19: 안내 문구가 탭 이름을 글로만 적어 어디인지 찾아야 했다 (알림 채널은 '서버 관리' 안의 채널 화면).
 * 화면마다 주소를 하나씩 두고, 안내 문구에는 누르면 그 화면이 열리는 링크를 붙인다.
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { LinkedText, ScreenLink } from '../components/ui/ScreenLink';
import '../i18n';
import { useServerStore } from '../stores/useServerStore';
import { useUIStore } from '../stores/useUIStore';
import { openScreen } from '../utils/openScreen';
import { parseScreen } from '../utils/screenAddress';

describe('화면 주소', () => {
    it('서버·디스코드·설정·작업 기록 주소를 읽는다', () => {
        expect(parseScreen('server/s-1/commands')).toEqual({ page: 'server', id: 's-1', tab: 'commands' });
        expect(parseScreen('server/s-1')).toEqual({ page: 'server', id: 's-1' });
        expect(parseScreen('discord')).toEqual({ page: 'discord', view: 'connection' });
        expect(parseScreen('discord/saba')).toEqual({ page: 'discord', view: 'saba' });
        expect(parseScreen('discord/123/moderation/channels')).toEqual({
            page: 'discord',
            view: 'node',
            id: '123',
            tab: 'moderation',
            sub: 'channels',
        });
        expect(parseScreen('discord/*/members')).toEqual({ page: 'discord', view: 'node', id: '*', tab: 'members' });
        expect(parseScreen('settings/connection')).toEqual({ page: 'settings', tab: 'connection' });
        expect(parseScreen('settings/sabaStorage/modules')).toEqual({ page: 'settings', tab: 'sabaStorage', sub: 'modules' });
        expect(parseScreen('activity')).toEqual({ page: 'activity' });
    });

    it('모르는 화면이나 빈 칸이 있는 주소는 읽지 않는다', () => {
        expect(parseScreen('')).toBeNull();
        expect(parseScreen('nowhere')).toBeNull();
        expect(parseScreen('server')).toBeNull();
        expect(parseScreen('server//console')).toBeNull();
        expect(parseScreen('server/a b/console')).toBeNull();
    });
});

describe('주소로 화면 열기', () => {
    beforeEach(() => {
        useUIStore.getState()._resetForTest();
        useServerStore.setState({
            servers: [{ id: 's-1', name: 'MC', module: 'minecraft', status: 'stopped' }] as never,
        });
        window.showToast = vi.fn() as never;
    });

    it('서버 탭을 연다', () => {
        openScreen('server/s-1/commands');
        const ui = useUIStore.getState();
        expect([ui.activeTab, ui.activeServerId, ui.requestedServerTab]).toEqual(['servers', 's-1', 'commands']);
        expect(window.showToast).not.toHaveBeenCalled();
    });

    it('없어진 서버면 서버 목록을 열고 이유를 알린다', () => {
        openScreen('server/gone/console');
        expect(useUIStore.getState().activeTab).toBe('servers');
        expect(useUIStore.getState().activeServerId).toBeNull();
        expect(window.showToast).toHaveBeenCalledWith(
            "That server can't be found, so the server list is open instead.",
            'info',
            expect.any(Number),
        );
    });

    it('디스코드 서버의 탭과 안 화면을 디스코드 페이지에 요청한다', () => {
        openScreen('discord/123/moderation/channels');
        const ui = useUIStore.getState();
        expect(ui.activeTab).toBe('discord');
        expect(ui.requestedDiscordView).toMatchObject({ kind: 'node', id: '123', tab: 'moderation', sub: 'channels' });
    });

    it('설정 탭을 연다', () => {
        openScreen('settings/connection');
        expect(useUIStore.getState().activeTab).toBe('settings');
        expect(useUIStore.getState().settingsInitialView).toBe('connection');
    });

    it('설정을 떠나면 열 탭 요청을 지워서 다음에 설정을 열 때 따라오지 않는다', () => {
        openScreen('settings/connection');
        useUIStore.getState().setActiveTab('servers');
        expect(useUIStore.getState().settingsInitialView).toBeNull();
    });
});

describe('링크', () => {
    beforeEach(() => {
        useUIStore.getState()._resetForTest();
    });

    it('지금 화면에서 가는 길을 보이고, 누르면 그 화면을 연다', () => {
        render(<ScreenLink to="discord/123/moderation/channels" />);
        const link = screen.getByRole('button', { name: 'Open Discord › Server tools › Channels' });
        expect(link.textContent).toBe('Discord›Server tools›Channels↗');
        fireEvent.click(link);
        expect(useUIStore.getState().requestedDiscordView).toMatchObject({ id: '123', tab: 'moderation' });
    });

    it('안내 문구의 <link/> 자리에 링크가 들어간다', () => {
        render(
            <p>
                <LinkedText i18nKey="discord.start_notice_hint" to="discord/123/moderation/channels" />
            </p>,
        );
        const p = document.querySelector('p') as HTMLElement;
        expect(p.textContent).toMatch(/Pick the alert channel in Discord›Server tools›Channels↗\.$/);
        expect(p.textContent).not.toContain('<link');
    });

    it('읽지 못한 주소는 링크 대신 아무것도 그리지 않는다', () => {
        const { container } = render(<ScreenLink to="nowhere" />);
        expect(container.textContent).toBe('');
    });
});

describe('디스코드 페이지가 요청한 화면을 연다', () => {
    it('없는 디스코드 서버면 연결 화면에 남고 이유를 알린다', async () => {
        const { DiscordPage } = await import('../components/discord/DiscordPage');
        window.showToast = vi.fn() as never;
        useUIStore.setState({ requestedDiscordView: { kind: 'node', id: 'nope', tab: 'members', seq: 1 } });
        const Page = DiscordPage as unknown as React.ComponentType<Record<string, unknown>>;
        await act(async () => {
            render(
                <Page
                    discordBotStatus="running"
                    discordCloudHostId="h"
                    setDiscordCloudHostId={() => {}}
                    relayConnected={false}
                    relayConnecting={false}
                    handleStartDiscordBot={() => {}}
                    saveCurrentSettings={() => {}}
                    cloudNodes={[{ guildId: 'g1', guildName: '사바 마을' }]}
                    setCloudNodes={() => {}}
                    setCloudMembers={() => {}}
                />,
            );
        });
        expect(window.showToast).toHaveBeenCalledWith(
            "That Discord server isn't connected, so the connection screen is open instead.",
            'info',
            expect.any(Number),
        );
        expect(useUIStore.getState().requestedDiscordView).toBeNull();
    });
});
