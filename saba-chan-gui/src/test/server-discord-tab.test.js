/** 서버마다 디스코드에서 부르는 말은 서버 상세의 디스코드 탭에서 고친다 (디스코드 화면에는 없다) */
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('../components/ExtensionSlot', () => ({ default: () => null }));
vi.mock('../components/servers/ServerConsole', () => ({ ServerConsole: () => null }));
vi.mock('../components/Modals/CommandModal', () => ({ default: () => null }));

import { ServerDetail } from '../components/servers/ServerDetail';
import { useDiscordStore } from '../stores/useDiscordStore';
import { useServerStore } from '../stores/useServerStore';

const server = { id: 's1', name: '야생', module: 'minecraft', status: 'stopped' };

const detail = () =>
    render(
        <ServerDetail
            server={server}
            moduleData={{ name: 'minecraft' }}
            modules={[]}
            servers={[server]}
            setServers={() => {}}
            onStart={() => {}}
            onStop={() => {}}
            setModal={() => {}}
            consoleProps={{ consoles: {}, popoutOpen: {} }}
            settingsProps={{}}
        />,
    );

describe('서버 상세의 디스코드 탭', () => {
    afterEach(() => {
        window.api = undefined;
    });

    it('페어링된 디스코드 서버마다 이 서버의 부르는 말을 보인다', async () => {
        useServerStore.setState({ modulesLoaded: true });
        useDiscordStore.setState({ discordCloudHostId: 'host-1' });
        window.api = {
            relayListHostNodes: vi.fn().mockResolvedValue([{ guildId: 'g1', guildName: '사바 마을' }]),
            guildInstances: vi
                .fn()
                .mockResolvedValue([{ id: 's1', name: '야생', module: 'minecraft', words: [], commands: [] }]),
            guildInstanceWordsSave: vi.fn(),
        };
        detail();
        await act(async () => fireEvent.click(screen.getByText('Discord')));
        expect(await screen.findByText('사바 마을')).toBeTruthy();
        expect(window.api.guildInstances).toHaveBeenCalledWith('g1');
    });

    it('데몬이 낱말 API를 모르면 탭을 두지 않는다', () => {
        useServerStore.setState({ modulesLoaded: true });
        window.api = {};
        detail();
        expect(screen.queryByText('Discord')).toBeNull();
    });
});
