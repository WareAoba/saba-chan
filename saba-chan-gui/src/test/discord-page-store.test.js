import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { DiscordPage } from '../components/discord/DiscordPage';
import { useDiscordStore } from '../stores/useDiscordStore';

const servers = [
    { id: 's1', name: '야생 서버', module: 'minecraft' },
    { id: 's2', name: '팰 서버', module: 'palworld' },
];
const modules = [
    { name: 'minecraft', game_name: 'Minecraft', commands: { fields: [] } },
    { name: 'palworld', game_name: 'Palworld', commands: { fields: [] } },
];

/** App처럼 값을 스토어에서 읽는다 */
function StoreHarness() {
    const s = useDiscordStore();
    const set = (key) => (v) =>
        useDiscordStore.getState().update({ [key]: typeof v === 'function' ? v(useDiscordStore.getState()[key]) : v });
    return (
        <DiscordPage
            discordBotStatus="running"
            discordPrefixes={[]}
            setDiscordPrefixes={() => {}}
            discordCloudHostId={s.discordCloudHostId}
            setDiscordCloudHostId={set('discordCloudHostId')}
            relayConnected
            relayConnecting={false}
            saveCurrentSettings={vi.fn().mockResolvedValue(true)}
            servers={servers}
            modules={modules}
            moduleAliasesPerModule={{}}
            discordModuleAliases={{}}
            discordCommandAliases={{}}
            setDiscordModuleAliases={() => {}}
            setDiscordCommandAliases={() => {}}
            nodeSettings={s.nodeSettings}
            setNodeSettings={set('nodeSettings')}
            cloudNodes={s.cloudNodes}
            setCloudNodes={set('cloudNodes')}
            cloudMembers={s.cloudMembers}
            setCloudMembers={set('cloudMembers')}
        />
    );
}

const deferred = () => {
    let resolve;
    const promise = new Promise((r) => {
        resolve = r;
    });
    return { promise, resolve };
};

describe('디스코드 화면과 데몬 설정', () => {
    beforeEach(() => {
        Object.defineProperty(window, 'innerWidth', { value: 1280, writable: true, configurable: true });
        window.api = {
            relayListHostNodes: vi.fn().mockResolvedValue([{ guildId: 'g1', guildName: '사바 마을' }]),
            relayListNodeMembers: vi.fn().mockResolvedValue([]),
            botConfigLoad: vi.fn().mockResolvedValue({ prefix: '!saba', cloud: { hostId: 'host-1' } }),
            botConfigSave: vi.fn().mockResolvedValue({ success: true }),
        };
        window.showToast = vi.fn();
    });

    it('화면을 열 때 데몬 설정을 다시 읽어 CLI로 짝지은 호스트를 보인다', async () => {
        useDiscordStore.setState({ _botConfigLoaded: true, discordCloudHostId: '' });
        window.api.botConfigLoad = vi.fn().mockResolvedValue({ prefix: '!saba', cloud: { hostId: 'h-cli' } });
        await act(async () => render(<StoreHarness />));
        await waitFor(() => expect(useDiscordStore.getState().discordCloudHostId).toBe('h-cli'));
    });

    it('허용 인스턴스를 빠르게 두 번 바꿔도 앞의 변경이 사라지지 않는다', async () => {
        useDiscordStore.setState({
            _botConfigLoaded: true,
            discordCloudHostId: 'host-1',
            nodeSettings: { g1: { allowedInstances: ['s1', 's2'] } },
        });
        const saves = [];
        window.api.guildAllowedInstancesSave = vi.fn((guildId, instances) => {
            const d = deferred();
            saves.push(() => d.resolve({ guildId, allowedInstances: instances }));
            return d.promise;
        });
        await act(async () => render(<StoreHarness />));
        await waitFor(() => expect(document.querySelectorAll('.dc-nav-row').length).toBeGreaterThan(1));
        const nodeRow = [...document.querySelectorAll('.dc-nav-row')].find((b) => b.textContent.includes('사바 마을'));
        await act(async () => fireEvent.click(nodeRow));

        await act(async () => fireEvent.click(screen.getByLabelText('야생 서버')));
        await act(async () => fireEvent.click(screen.getByLabelText('팰 서버')));
        await act(async () => saves.shift()());
        await waitFor(() => expect(saves).toHaveLength(1));
        await act(async () => saves.shift()());

        expect(window.api.guildAllowedInstancesSave.mock.calls.map((c) => c[1])).toEqual([['s2'], []]);
        expect(useDiscordStore.getState().nodeSettings.g1.allowedInstances).toEqual([]);
    });
});
