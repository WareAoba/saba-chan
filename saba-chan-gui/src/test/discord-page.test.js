import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { useState } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { cloudStateOf, DiscordPage } from '../components/discord/DiscordPage';
import * as P from '../components/discord/nodePermissions';

const NODES = ['g1', 'g2'];

describe('nodePermissions (데몬 src/chat/mod.rs 해석과 같다)', () => {
    it('설정이 없는 노드는 모든 인스턴스 허용', () => {
        expect(P.instanceMode({}, 'g1')).toBe('all');
        expect(P.isInstanceAllowed({}, 'g1', 'a')).toBe(true);
        expect(P.isInstanceAllowed({ g1: { allowedInstances: [] } }, 'g1', 'a')).toBe(false);
    });

    it("'고른 것만'으로 바꾸면 지금 보이던 것을 그대로 고르되 다른 노드 것은 뺀다", () => {
        const s = { g2: { allowedInstances: ['b'] } };
        const next = P.setInstanceMode(s, 'g1', 'pick', ['a', 'b', 'c'], NODES);
        expect(next.g1.allowedInstances).toEqual(['a', 'c']);
        expect(P.setInstanceMode(next, 'g1', 'all', [], NODES).g1).not.toHaveProperty('allowedInstances');
    });

    it('다른 노드가 고른 인스턴스는 켤 수 없고, 끄기는 언제나 된다', () => {
        const s = { g1: { allowedInstances: ['a'] }, g2: { allowedInstances: ['b'] } };
        expect(P.toggleInstance(s, 'g1', 'b', NODES)).toBe(s);
        expect(P.toggleInstance(s, 'g1', 'c', NODES).g1.allowedInstances).toEqual(['a', 'c']);
        expect(P.toggleInstance(s, 'g1', 'a', NODES).g1.allowedInstances).toEqual([]);
        expect(P.instanceOwner(s, NODES, 'b', 'g1')).toBe('g2');
    });

    it('멤버 제한은 막힌 명령 배열로 저장된다', () => {
        let s = P.setMemberRestricted({}, 'g1', 'u', true);
        expect(P.isMemberRestricted(s, 'g1', 'u')).toBe(true);
        s = P.toggleMemberCommand(s, 'g1', 'u', 'a', 'stop');
        expect(s.g1.memberPermissions.u.a).toEqual(['stop']);
        expect(P.blockedCount(s, 'g1', 'u')).toBe(1);
        s = P.setMemberAllCommands(s, 'g1', 'u', 'a', ['stop', 'say'], false);
        expect(s.g1.memberPermissions.u.a).toEqual(['stop', 'say']);
        s = P.setMemberAllCommands(s, 'g1', 'u', 'a', ['stop', 'say'], true);
        expect(s.g1.memberPermissions.u.a).toEqual([]);
        s = P.setMemberRestricted(s, 'g1', 'u', false);
        expect(P.isMemberRestricted(s, 'g1', 'u')).toBe(false);
    });

    it('별명 초안: 저장된 값이 있으면 그것, 없으면 module.toml 기본값', () => {
        const defaults = { module_aliases: ['mc'], commands: { start: { aliases: ['시작'] }, stop: { aliases: [] } } };
        const d = P.aliasDraftFor('minecraft', defaults, { minecraft: '마크, 마인' }, { minecraft: { stop: '끄기' } });
        expect(d.moduleAliases).toEqual(['마크', '마인']);
        expect(d.commands.start.aliases).toEqual(['시작']);
        expect(d.commands.stop.aliases).toEqual(['끄기']);
        expect(P.aliasDraftFor('minecraft', defaults, {}, {}).moduleAliases).toEqual(['mc']);
    });

    it('한 모듈 안에서 두 명령이 같은 단어를 쓰면 찾아낸다', () => {
        expect(P.commandAliasClashes({ start: { aliases: ['켜'] }, stop: { aliases: ['켜'] } })).toHaveLength(1);
        expect(P.commandAliasClashes({ start: { aliases: ['stop'] }, stop: { aliases: [] } })).toHaveLength(1);
        expect(P.commandAliasClashes({ start: { aliases: ['켜'] }, stop: { aliases: ['꺼'] } })).toHaveLength(0);
    });

    it('연결 상태', () => {
        expect(cloudStateOf({ hostId: '' })).toBe('no_host');
        expect(cloudStateOf({ hostId: 'h', connected: true })).toBe('connected');
        expect(cloudStateOf({ hostId: 'h', error: 'x' })).toBe('error');
        expect(cloudStateOf({ hostId: 'h' })).toBe('connecting');
    });
});

const servers = [
    { id: 's1', name: '야생 서버', module: 'minecraft' },
    { id: 's2', name: '팰 서버', module: 'palworld' },
];
const modules = [
    { name: 'minecraft', game_name: 'Minecraft', commands: { fields: [{ name: 'stop', risk: 'danger' }] } },
    { name: 'palworld', game_name: 'Palworld', commands: { fields: [] } },
];
const aliases = {
    minecraft: {
        module_aliases: ['mc'],
        commands: { start: { aliases: ['시작'], label: '시작' }, stop: { aliases: [], label: '정지' } },
    },
    palworld: { module_aliases: ['pal'], commands: {} },
};

function Harness({ save, initialView, botConfigSave }) {
    const [nodeSettings, setNodeSettings] = useState({});
    const [cloudNodes, setCloudNodes] = useState([]);
    const [cloudMembers, setCloudMembers] = useState({});
    const [ma, setMa] = useState({});
    const [ca, setCa] = useState({});
    window.__nodeSettings = nodeSettings;
    window.api.botConfigSave = botConfigSave;
    return (
        <DiscordPage
            discordBotStatus="running"
            discordPrefix="!saba"
            setDiscordPrefix={() => {}}
            discordCloudHostId="host-1"
            setDiscordCloudHostId={() => {}}
            relayConnected
            relayConnecting={false}
            saveCurrentSettings={save}
            servers={servers}
            modules={modules}
            moduleAliasesPerModule={aliases}
            discordModuleAliases={ma}
            discordCommandAliases={ca}
            setDiscordModuleAliases={setMa}
            setDiscordCommandAliases={setCa}
            nodeSettings={nodeSettings}
            setNodeSettings={(v) => setNodeSettings((p) => (typeof v === 'function' ? v(p) : v))}
            cloudNodes={cloudNodes}
            setCloudNodes={setCloudNodes}
            cloudMembers={cloudMembers}
            setCloudMembers={(v) => setCloudMembers((p) => (typeof v === 'function' ? v(p) : v))}
            initialView={initialView}
        />
    );
}

describe('DiscordPage', () => {
    beforeEach(() => {
        Object.defineProperty(window, 'innerWidth', { value: 1280, writable: true, configurable: true });
        window.api = {
            relayListHostNodes: vi.fn().mockResolvedValue([{ guildId: 'g1', guildName: '사바 마을' }]),
            relayListNodeMembers: vi.fn().mockResolvedValue([{ id: 'u1', username: 'mint', displayName: '민트' }]),
            botConfigLoad: vi.fn().mockResolvedValue({ prefix: '!saba', moduleAliases: {}, commandAliases: {} }),
        };
        window.showToast = vi.fn();
    });

    it('노드 → 멤버 → 명령 하나 막기가 막힌 명령 배열로 들어가고 자동 저장된다', async () => {
        const save = vi.fn().mockResolvedValue();
        await act(async () => render(<Harness save={save} />));
        await waitFor(() => expect(document.querySelectorAll('.dc-nav-row')).toHaveLength(4));

        // 메뉴의 노드를 누르면 노드 화면
        const nodeRow = [...document.querySelectorAll('.dc-nav-row')].find((b) => b.textContent.includes('사바 마을'));
        await act(async () => fireEvent.click(nodeRow));
        expect(document.querySelector('.detail-head h1').textContent).toBe('사바 마을');

        // 멤버 탭 → 멤버 → 명령 골라서 허용
        await act(async () => fireEvent.click(document.querySelectorAll('.segmented-item')[1]));
        await waitFor(() => expect(document.querySelector('.dc-list-btn')).not.toBeNull());
        await act(async () => fireEvent.click(document.querySelector('.dc-list-btn')));
        await act(async () => fireEvent.click(document.querySelectorAll('.dc-radio-input')[1]));

        // 제한 없는 노드이므로 인스턴스 둘 다 보이고, 정지(위험) 명령을 끈다
        expect(document.querySelectorAll('.dc-perm-card')).toHaveLength(2);
        const stop = [...document.querySelectorAll('.dc-cmd')].find((b) => b.textContent.includes('정지'));
        expect(stop.classList.contains('danger')).toBe(true);
        await act(async () => fireEvent.click(stop));

        expect(window.__nodeSettings.g1.memberPermissions.u1.s1).toEqual(['stop']);
        await waitFor(() => expect(save).toHaveBeenCalled(), { timeout: 2000 });
    });

    it('명령어 단어: 칩을 추가하고 저장하면 쉼표 문자열로 보낸다', async () => {
        const botConfigSave = vi.fn().mockResolvedValue({});
        await act(async () =>
            render(
                <Harness
                    save={vi.fn()}
                    botConfigSave={botConfigSave}
                    initialView={{ kind: 'words', module: 'minecraft' }}
                />,
            ),
        );
        const input = document.querySelector('.word-input input');
        await act(async () => fireEvent.change(input, { target: { value: '마크' } }));
        await act(async () => fireEvent.keyDown(input, { key: 'Enter' }));
        const saveBtn = document.querySelector('.dc-savebar .btn-primary');
        expect(saveBtn.disabled).toBe(false);
        await act(async () => fireEvent.click(saveBtn));

        expect(botConfigSave).toHaveBeenCalledWith(
            expect.objectContaining({
                moduleAliases: { minecraft: 'mc,마크' },
                commandAliases: { minecraft: { start: '시작', stop: '' } },
            }),
        );
    });
});
