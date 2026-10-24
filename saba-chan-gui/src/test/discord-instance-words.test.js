/**
 * 길드·인스턴스별 부르는 말 (계약 integration/discord-instance-commands.md §7).
 * 서버 상세의 디스코드 탭: 페어링된 디스코드 서버마다 그 서버의 낱말을 고친다.
 * 데몬 GET /api/relay/host/:host/nodes, GET /api/guilds/:guild/instances, PUT .../instances/:id/words.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { errorsByField, InstanceDiscordTab } from '../components/discord/InstanceWordsTab';
import { useDiscordStore } from '../stores/useDiscordStore';

const server = { id: 's1', name: '야생', module: 'minecraft' };
const nodes = [
    { guildId: 'g1', guildName: '사바 마을' },
    { guildId: 'g2', guildName: '옆 마을' },
];
const items = () => [
    {
        id: 's1',
        name: '야생',
        module: 'minecraft',
        words: ['크리'],
        commands: [
            { name: 'start', label: 'Start', risk: 'safe', builtin: true, moduleAliases: ['시작'], aliases: [] },
            { name: 'kick', label: 'Kick', risk: 'caution', builtin: false, moduleAliases: [], aliases: ['추방'] },
        ],
    },
];

async function addChip(label, word) {
    const input = screen.getByLabelText(label);
    await act(async () => fireEvent.change(input, { target: { value: word } }));
    await act(async () => fireEvent.keyDown(input, { key: 'Enter' }));
}

describe('InstanceDiscordTab', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        useDiscordStore.setState({ discordCloudHostId: 'host-1', discordCloudRelayUrl: '' });
        window.api = {
            relayListHostNodes: vi.fn().mockResolvedValue(nodes),
            // g2는 이 서버를 쓰지 않는다 (allowedInstances)
            guildInstances: vi.fn(async (g) => (g === 'g1' ? items() : [])),
            // 데몬 응답 모양: 빈 명령 목록은 빠진다
            guildInstanceWordsSave: vi.fn(async (_g, id, body) => ({
                success: true,
                id,
                words: body.words,
                commands: Object.fromEntries(Object.entries(body.commands).filter(([, l]) => l.length)),
            })),
        };
    });

    it('디스코드 서버마다 이 서버의 부르는 말과 명령 낱말을 배열로 저장한다', async () => {
        await act(async () => render(<InstanceDiscordTab server={server} />));
        await screen.findByText('사바 마을');
        expect(window.api.relayListHostNodes).toHaveBeenCalledWith('host-1', '');
        expect(window.api.guildInstances).toHaveBeenCalledWith('g1');
        expect(window.api.guildInstances).toHaveBeenCalledWith('g2');
        // 이 서버를 쓰지 않는 디스코드 서버는 이름과 안내만 보인다
        expect(screen.getByText('옆 마을')).toBeTruthy();
        expect(screen.getAllByLabelText('Names for 야생')).toHaveLength(1);
        await addChip('Names for 야생', '야섭');
        await addChip('Kick', '킥');
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Save/ })));
        expect(window.api.guildInstanceWordsSave).toHaveBeenCalledWith('g1', 's1', {
            words: ['크리', '야섭'],
            commands: { start: [], kick: ['추방', '킥'] },
        });
        // 저장한 값이 화면의 기준이 되어 더 고칠 것이 없다
        await waitFor(() => expect(screen.queryByRole('button', { name: /Save/ })).toBeNull());
        expect(screen.getByText('킥')).toBeTruthy();
    });

    it('검증 오류는 그 칸 아래에 겹치는 대상과 함께 보인다', async () => {
        window.api.guildInstanceWordsSave = vi.fn().mockResolvedValue({
            ok: false,
            error_code: 'validation_failed',
            details: [{ field: 'words', reason: 'conflict', word: '팰', conflictWith: 'palworld' }],
        });
        await act(async () => render(<InstanceDiscordTab server={server} />));
        await screen.findByText('사바 마을');
        await addChip('Names for 야생', '팰');
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Save/ })));
        await waitFor(() => expect(screen.getByText(/palworld/)).toBeTruthy());
        expect(document.querySelector('.word-chip.bad').textContent).toContain('팰');
    });

    it('목록을 못 받으면 이유와 다시 시도를 보인다', async () => {
        window.api.guildInstances = vi.fn().mockResolvedValue({ ok: false, error: 'relay down' });
        await act(async () => render(<InstanceDiscordTab server={server} />));
        expect(await screen.findByText(/relay down/)).toBeTruthy();
    });

    it('디스코드에 연결하지 않았으면 목록을 부르지 않고 안내만 보인다', async () => {
        useDiscordStore.setState({ discordCloudHostId: '' });
        await act(async () => render(<InstanceDiscordTab server={server} />));
        expect(window.api.relayListHostNodes).not.toHaveBeenCalled();
        expect(document.querySelector('.dc-tab .muted')).toBeTruthy();
    });

    it('details의 field를 칸별로 나눈다', () => {
        expect(
            Object.keys(
                errorsByField([{ field: 'words[1]' }, { field: 'commands.kick' }, { field: 'commands.kick[0]' }]),
            ),
        ).toEqual(['words', 'commands.kick']);
    });
});
