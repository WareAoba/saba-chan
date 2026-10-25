import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as L from '../components/discord/levels';
import { NodeView } from '../components/discord/NodeView';
import { OWNER_CONFIRM_MS, PermissionPanel } from '../components/discord/PermissionsTab';
import { pickOption } from './dropdown';

const API = ['permissionsGet', 'permissionsUpdate', 'permissionsSetMember', 'permissionsRemoveMember'];
const node = { guildId: 'g1', guildName: 'Guild' };
const servers = [{ id: 'mc-1', name: 'Survival', module: 'minecraft' }];
const relayMembers = [
    { id: '1001', username: 'aoba', displayName: 'Aoba', level: 2 },
    { id: '1002', username: 'mint', displayName: 'Mint', level: 2 },
    { id: '1003', username: 'newbie', displayName: 'Newbie', level: 0 },
];
const base = (over) => ({
    guildId: 'g1',
    ownerDiscordId: '1001',
    defaultLevel: 1,
    preset: 'default',
    chatbotCap: 0,
    members: { 1002: 2, 1004: -1 },
    commandLevels: {},
    ...over,
});
const flush = () => act(async () => {});

function renderPanel(tab = 'members', props = {}) {
    return render(
        <PermissionPanel
            tab={tab}
            node={node}
            servers={servers}
            members={relayMembers}
            commandsFor={() => [{ name: 'say', label: 'Say', description: '' }]}
            riskOf={() => null}
            {...props}
        />,
    );
}

describe('권한 레벨 계산', () => {
    it('명령 요구 레벨은 덮어쓴 값, 없으면 프리셋표[risk] (risk 없으면 caution)', () => {
        const perm = base({ preset: 'strict', commandLevels: { 'mc-1/stop': 0 } });
        expect(L.requiredLevel(perm, 'mc-1', 'stop', 'caution')).toEqual({ level: 0, overridden: true });
        expect(L.requiredLevel(perm, 'mc-1', 'start', 'caution')).toEqual({ level: 2, overridden: false });
        expect(L.requiredLevel(base(), 'mc-1', 'say', undefined).level).toBe(1);
        expect(L.presetLevel(base({ preset: 'loose' }), 'danger')).toBe(1);
    });

    it('멤버는 소유자부터 레벨별로 묶고, 소유자는 members에 있어도 소유자로만 둔다', () => {
        const groups = L.membersByLevel(base({ members: { 1001: 0, 1002: 2, 1004: -1 } }));
        expect(groups).toEqual([
            { level: 3, ids: ['1001'] },
            { level: 2, ids: ['1002'] },
            { level: -1, ids: ['1004'] },
        ]);
    });

    it('길드 사다리: 더한 단계는 이름으로, 지운 기본 단계는 프리셋에서 더 엄한 단계로 푼다', () => {
        const perm = base({
            ladder: [{ id: 3 }, { id: 10, name: 'VIP' }, { id: 2 }, { id: 0 }, { id: -1 }],
            members: { 1002: 10, 1004: -1 },
        });
        expect(L.grantable(perm)).toEqual([10, 2, 0, -1]);
        expect(L.requirable(perm)).toEqual([3, 10, 2, 0]);
        expect(L.presetLevel(perm, 'caution')).toBe(2);
        expect(L.membersByLevel(perm).map((g) => g.level)).toEqual([3, 10, -1]);
        expect(L.ladderOf(base()).map((l) => l.id)).toEqual([3, 2, 1, 0, -1]);
    });

    it('사다리 편집: 소유자는 맨 위, 금지는 맨 아래에 남는다', () => {
        const ladder = L.ladderOf(base());
        expect(L.moveLevel(ladder, 3, 1).map((l) => l.id)).toEqual([3, 0, 2, 1, -1]);
        expect(L.moveLevel(ladder, 3, 0)).toBe(ladder);
        expect(L.moveLevel(ladder, 0, 2)).toBe(ladder);
        expect(L.moveLevel(ladder, 1, 4)).toBe(ladder);
        expect(L.ladderPatch(L.addLevel(ladder, ' Mod ')).slice(0, 2)).toEqual([{ id: 3 }, { name: 'Mod' }]);
    });

    it('기본 레벨 멤버는 디스코드 멤버 중 소유자도 아니고 레벨도 없는 사람', () => {
        expect(L.defaultMembers(base(), relayMembers).map((m) => m.id)).toEqual(['1003']);
        expect(L.defaultMembers(base(), undefined)).toEqual([]);
    });

    it('가져오기 후보는 데몬 값과 다른 릴레이 부여값만 (소유자 제외)', () => {
        expect(L.importCandidates(base(), relayMembers).map((m) => m.id)).toEqual(['1003']);
    });

    it('릴레이 소유자가 고정된 소유자와 다를 때만 불일치', () => {
        expect(L.ownerMismatch(base())).toBeNull();
        expect(L.ownerMismatch(base({ relayOwnerDiscordId: '1001' }))).toBeNull();
        expect(L.ownerMismatch(base({ relayOwnerDiscordId: '1009' }))).toBe('1009');
    });
});

describe('권한 화면', () => {
    let perm;
    beforeEach(() => {
        perm = base();
        window.showToast = vi.fn();
        window.api.permissionsGet = vi.fn(async () => structuredClone(perm));
        window.api.permissionsUpdate = vi.fn(async () => ({ success: true }));
        window.api.permissionsSetMember = vi.fn(async () => ({ success: true }));
        window.api.permissionsRemoveMember = vi.fn(async () => ({ success: true }));
    });
    afterEach(() => {
        for (const k of API) delete window.api[k];
        delete window.api.relayListLegacyMembers;
        window.showToast = undefined;
    });

    it('레벨별로 멤버를 보이고, 바꾸면 데몬에 쓰고 다시 읽는다', async () => {
        renderPanel();
        await flush();
        expect(window.api.permissionsGet).toHaveBeenCalledWith('g1');
        const owner = screen.getByRole('region', { name: 'Owner' });
        expect(within(owner).getByText('Aoba')).toBeInTheDocument();
        expect(within(owner).queryByRole('combobox')).toBeNull();
        expect(within(screen.getByRole('region', { name: 'Banned' })).getByText('1004')).toBeInTheDocument();
        expect(screen.getByText('Everyone else uses the default level (User).')).toBeInTheDocument();

        pickOption(screen.getByLabelText('Level for Mint'), 'Banned');
        await flush();
        expect(window.api.permissionsSetMember).toHaveBeenCalledWith('g1', '1002', -1);
        expect(window.api.permissionsGet).toHaveBeenCalledTimes(2);

        pickOption(screen.getByLabelText('Default level'), 'Talking Saba-chan');
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', { defaultLevel: 0 });

        fireEvent.click(screen.getByRole('tab', { name: 'Strict' }));
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', { preset: 'strict' });
    });

    it('레벨이 없는 디스코드 멤버도 기본 레벨 묶음에 보이고, 거기서 레벨을 줄 수 있다', async () => {
        renderPanel();
        await flush();
        const others = screen.getByRole('region', { name: 'Members on the default level' });
        expect(within(others).getByText('Newbie')).toBeInTheDocument();
        expect(within(others).queryByText('Mint')).toBeNull();

        pickOption(screen.getByLabelText('Level for Newbie'), 'Admin');
        await flush();
        expect(window.api.permissionsSetMember).toHaveBeenCalledWith('g1', '1003', 2);
    });

    it('디스코드 멤버를 못 받았으면 레벨 목록 아래에 그렇다고 알린다', async () => {
        renderPanel('members', { members: [] });
        await flush();
        expect(screen.getByText(/Couldn't load members/)).toBeInTheDocument();
        expect(screen.queryByRole('region', { name: 'Members on the default level' })).toBeNull();
    });

    it('데몬이 거절하면 알림을 띄우고 화면은 데몬 값 그대로 둔다', async () => {
        window.api.permissionsSetMember = vi.fn(async () => ({ error: 'x', error_code: 'owner_fixed' }));
        renderPanel();
        await flush();
        pickOption(screen.getByLabelText('Level for Mint'), 'User');
        await flush();
        expect(window.showToast).toHaveBeenCalledWith("The owner's level can't be changed.", 'error', 5000, undefined);
        expect(screen.getByLabelText('Level for Mint')).toHaveTextContent('Admin');
    });

    it('릴레이에 남은 부여값(legacy-members)을 골라 가져온다: 디스코드 멤버 목록에는 레벨이 없다', async () => {
        window.api.relayListLegacyMembers = vi.fn(async () => relayMembers);
        renderPanel('members', { members: relayMembers.map(({ level, ...m }) => m) });
        await flush();
        expect(window.api.relayListLegacyMembers).toHaveBeenCalledWith('g1');
        fireEvent.click(screen.getByRole('checkbox', { name: 'Newbie' }));
        fireEvent.click(screen.getByRole('button', { name: 'Import 1' }));
        await flush();
        expect(window.api.permissionsSetMember).toHaveBeenCalledWith('g1', '1003', 0);
    });

    it('새로고침하면 릴레이에 남은 부여값도 다시 읽는다', async () => {
        window.api.relayListLegacyMembers = vi.fn(async () => relayMembers);
        renderPanel('members', { members: relayMembers.map(({ level, ...m }) => m), onRefreshMembers: vi.fn() });
        await flush();
        expect(screen.getByRole('checkbox', { name: 'Newbie' })).toBeInTheDocument();

        fireEvent.click(screen.getByRole('checkbox', { name: 'Newbie' }));
        let release;
        window.api.relayListLegacyMembers.mockReturnValue(new Promise((r) => (release = r)));
        fireEvent.click(screen.getByRole('button', { name: /Refresh/ }));
        // 응답 전에는 옛 후보로 가져올 수 없다
        await flush();
        expect(screen.queryByRole('button', { name: /^Import/ })).toBeNull();
        await act(async () => release(relayMembers));
        expect(screen.getByRole('checkbox', { name: 'Newbie' })).not.toBeChecked();

        window.api.relayListLegacyMembers.mockResolvedValue([]);
        fireEvent.click(screen.getByRole('button', { name: /Refresh/ }));
        await flush();
        expect(window.api.relayListLegacyMembers).toHaveBeenCalledTimes(3);
        expect(screen.queryByRole('checkbox', { name: 'Newbie' })).toBeNull();
    });

    it('소유자 불일치를 알리고, 고르면 소유자를 바꾼다', async () => {
        perm = base({ relayOwnerDiscordId: '1002' });
        renderPanel();
        await flush();
        expect(screen.getByRole('alert')).toHaveTextContent(
            'The relay reported a different owner (Mint). The owner here stays Aoba.',
        );
        vi.useFakeTimers({ shouldAdvanceTime: true });
        fireEvent.click(screen.getByRole('button', { name: 'Use this owner' }));
        await flush();
        expect(window.api.permissionsUpdate).not.toHaveBeenCalled(); // 한 번 더 눌러야 바꾼다
        // 확인 상태는 잠깐 뒤 풀린다
        await act(async () => {
            vi.advanceTimersByTime(OWNER_CONFIRM_MS);
        });
        vi.useRealTimers();
        fireEvent.click(screen.getByRole('button', { name: 'Use this owner' }));
        await flush();
        fireEvent.click(screen.getByRole('button', { name: 'Really make Mint (1002) the owner' }));
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', { ownerDiscordId: '1002' });

        fireEvent.click(screen.getByRole('button', { name: 'Change' }));
        fireEvent.change(screen.getByRole('textbox', { name: 'Owner' }), { target: { value: '123456789012345678' } });
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', { ownerDiscordId: '123456789012345678' });
    });

    it('명령 탭: 실제 레벨만 보이고, 고르면 그 명령을 덮어쓴다', async () => {
        perm = base({ commandLevels: { 'mc-1/say': 3 } });
        renderPanel('commands');
        await flush();
        const stop = screen.getByLabelText('Level needed for Stop');
        expect(stop).toHaveTextContent('User');
        expect(screen.getByLabelText('Level needed for Say')).toHaveTextContent('Owner');
        fireEvent.click(stop);
        expect(screen.queryByRole('option', { name: /Preset/ })).toBeNull();
        pickOption(screen.getByLabelText('Level needed for Stop'), 'Admin');
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', { commandLevels: { 'mc-1/stop': 2 } });
    });

    it('명령별 레벨이 있으면 프리셋을 바꾸기 전에 다시 맞춘다고 묻는다', async () => {
        perm = base({ commandLevels: { 'mc-1/say': 3 } });
        renderPanel();
        await flush();
        fireEvent.click(screen.getByRole('tab', { name: 'Strict' }));
        await flush();
        expect(window.api.permissionsUpdate).not.toHaveBeenCalled();
        expect(screen.getByRole('alert')).toHaveTextContent('1 command level(s) will be reset to the Strict preset.');
        fireEvent.click(screen.getByRole('button', { name: 'Apply' }));
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', { preset: 'strict' });
    });

    it('더한 단계도 멤버 레벨로 고를 수 있다', async () => {
        perm = base({
            ladder: [{ id: 3 }, { id: 2 }, { id: 10, name: 'VIP' }, { id: 1 }, { id: 0 }, { id: -1 }],
            members: { 1002: 10 },
        });
        renderPanel();
        await flush();
        expect(within(screen.getByRole('region', { name: 'VIP' })).getByText('Mint')).toBeInTheDocument();
        pickOption(screen.getByLabelText('Level for Newbie'), 'VIP');
        await flush();
        expect(window.api.permissionsSetMember).toHaveBeenCalledWith('g1', '1003', 10);
    });

    it('단계 탭: 블록을 끌어 순서를 바꾸고, 더하고 지운 뒤 저장한다', async () => {
        perm = base({ ladder: [{ id: 3 }, { id: 2 }, { id: 10, name: 'VIP' }, { id: 1 }, { id: 0 }, { id: -1 }] });
        renderPanel('levels');
        await flush();
        const block = (name) => screen.getByRole('listitem', { name });
        expect(within(block('Owner')).queryByRole('button', { name: /Delete/ })).toBeNull();
        expect(block('Banned')).not.toHaveAttribute('draggable', 'true');

        fireEvent.dragStart(block('VIP'));
        fireEvent.dragOver(block('Admin'));
        fireEvent.drop(block('Admin'));
        fireEvent.click(screen.getByRole('button', { name: 'Delete User' }));
        expect(screen.getByText(/Members of deleted levels move/)).toBeInTheDocument();
        fireEvent.change(screen.getByRole('textbox', { name: 'New level name' }), { target: { value: 'Mod' } });
        fireEvent.click(screen.getByRole('button', { name: 'Add' }));
        fireEvent.click(screen.getByRole('button', { name: 'Move Mod down' }));
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', {
            ladder: [{ id: 3 }, { id: 10, name: 'VIP' }, { name: 'Mod' }, { id: 2 }, { id: 0 }, { id: -1 }],
        });
    });

    it('preload에 권한 채널이 없으면 예전 멤버 화면을 쓰고 명령 탭이 없다', () => {
        for (const k of API) window.api[k] = undefined;
        render(
            <NodeView
                node={node}
                nodes={[node]}
                servers={servers}
                settings={{}}
                update={vi.fn()}
                members={relayMembers}
                onOpenMember={vi.fn()}
                commandsFor={() => []}
                riskOf={() => null}
            />,
        );
        expect(screen.queryByRole('tab', { name: 'Commands' })).toBeNull();
        fireEvent.click(screen.getByRole('tab', { name: 'Members' }));
        expect(screen.getAllByText('All commands').length).toBeGreaterThan(0);
    });
});
