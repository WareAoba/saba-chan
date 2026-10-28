import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as L from '../components/discord/levels';
import { NodeView } from '../components/discord/NodeView';
import { OWNER_CONFIRM_MS, PermissionPanel } from '../components/discord/PermissionsTab';
import { pickOption } from './dropdown';

const API = ['permissionsGet', 'permissionsCommands', 'permissionsUpdate', 'permissionsSetMember', 'permissionsRemoveMember'];
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
/** 데몬 `GET /api/permissions/:guildId/commands` 모양 */
const stair = (instance, name, title, level, over) => ({
    key: instance ? `${instance}/${name}` : name,
    keys: [instance ? `${instance}/${name}` : name],
    instance,
    name,
    title,
    kind: 'command',
    risk: 'caution',
    level,
    preset: level,
    overridden: false,
    fixed: false,
    ...over,
});
const baseBoard = () => ({
    guildId: 'g1',
    instances: [
        { id: 'mc-1', name: 'Survival', module: 'minecraft' },
        { id: 'mc-2', name: 'Creative', module: 'minecraft' },
    ],
    items: [
        stair('mc-1', 'stop', 'Stop', 1, { keys: ['mc-1/stop', 'mc-1/server.stop'] }),
        stair('mc-2', 'stop', 'Stop', 2, { keys: ['mc-2/stop', 'mc-2/server.stop'], preset: 1, overridden: true }),
        stair('mc-1', 'say', 'Say', 0, { risk: 'safe' }),
        stair('mc-2', 'say', 'Say', 0, { risk: 'safe' }),
        stair(null, 'instance.create', 'Create server', 2, { kind: 'admin' }),
        stair(null, 'update.apply', 'Update', 3, { kind: 'admin', risk: 'danger', fixed: true }),
    ],
});
const flush = () => act(async () => {});

function renderPanel(props = {}) {
    return render(<PermissionPanel node={node} members={relayMembers} {...props} />);
}

/** 인스턴스 메뉴의 레벨별 명령 화면 (처음 열리는 화면) */
async function openCommands(props) {
    renderPanel(props);
    await flush();
}

/** 인스턴스 메뉴의 멤버별 레벨 화면으로 */
async function openPeople(props) {
    renderPanel(props);
    await flush();
    fireEvent.click(screen.getByRole('tab', { name: 'Member levels' }));
    await flush();
}

const step = (name) => screen.getByRole('listitem', { name });
const level = (name) => screen.getByRole('listitem', { name });

describe('권한 레벨 계산', () => {
    it('레벨 번호는 금지(Lv 0)부터 위로 센다: 더한 레벨이 있으면 밀린다', () => {
        expect(L.levelNumber(base(), 3)).toBe(4);
        expect(L.levelNumber(base(), -1)).toBe(0);
        const perm = base({ ladder: [{ id: 3 }, { id: 10, name: 'VIP' }, { id: 2 }, { id: 1 }, { id: 0 }, { id: -1 }] });
        expect(L.levelNumber(perm, 3)).toBe(5);
        expect(L.ascending(perm)).toEqual([-1, 0, 1, 2, 10, 3]);
    });

    it('계단 칩: 같은 명령은 한 칩, 모든 인스턴스 보기에서 레벨이 다른 인스턴스는 꼬리표 칩', () => {
        const chips = L.stairChips(baseBoard());
        const stop = chips.filter((c) => c.name === 'stop');
        expect(stop.map((c) => [c.level, c.instance, c.items.length])).toEqual([
            [1, undefined, 2],
            [2, 'mc-2', 1],
        ]);
        expect(chips.find((c) => c.name === 'say').items).toHaveLength(2);
        // 한 인스턴스 보기: 그 인스턴스와 인스턴스를 받지 않는 것만
        const one = L.stairChips(baseBoard(), 'mc-2');
        expect(one.map((c) => [c.name, c.level])).toEqual([
            ['stop', 2],
            ['say', 0],
            ['instance.create', 2],
            ['update.apply', 3],
        ]);
    });

    it('칩 옮기기: 프리셋 값이면 덮어쓰기를 지우고, 아니면 첫 키에 쓰고 예전 키는 지운다', () => {
        const stop = L.stairChips(baseBoard()).find((c) => c.name === 'stop' && !c.instance);
        expect(L.levelPatch(stop.items, 2)).toEqual({
            'mc-1/stop': 2,
            'mc-1/server.stop': null,
            'mc-2/stop': 2,
            'mc-2/server.stop': null,
        });
        expect(L.levelPatch(stop.items, 1)['mc-2/stop']).toBeNull();
        const update = baseBoard().items.find((i) => i.fixed);
        expect(L.levelPatch([update], 1)).toEqual({});
    });

    it('멤버는 소유자부터 레벨별로 묶고, 소유자는 members에 있어도 소유자로만 둔다', () => {
        const groups = L.membersByLevel(base({ members: { 1001: 0, 1002: 2, 1004: -1 } }));
        expect(groups).toEqual([
            { level: 3, ids: ['1001'] },
            { level: 2, ids: ['1002'] },
            { level: -1, ids: ['1004'] },
        ]);
    });

    it('길드 사다리: 더한 레벨은 이름으로 쓴다', () => {
        const perm = base({
            ladder: [{ id: 3 }, { id: 10, name: 'VIP' }, { id: 2 }, { id: 0 }, { id: -1 }],
            members: { 1002: 10, 1004: -1 },
        });
        expect(L.grantable(perm)).toEqual([10, 2, 0, -1]);
        expect(L.requirable(perm)).toEqual([3, 10, 2, 0]);
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

    it('사다리 검사: 이름은 1~32자, 더한 단계는 20개까지 (데몬 invalid_ladder 와 같은 규칙)', () => {
        const ladder = L.ladderOf(base());
        expect(L.ladderValid(ladder)).toBe(true);
        expect(L.ladderValid(L.addLevel(ladder, '  '))).toBe(false);
        expect(L.ladderValid(L.addLevel(ladder, 'x'.repeat(33)))).toBe(false);
        let many = ladder;
        for (let i = 0; i < 20; i++) many = L.addLevel(many, `L${i}`);
        expect(L.ladderValid(many)).toBe(true);
        expect(L.canAddLevel(many)).toBe(false);
        expect(L.ladderValid(L.addLevel(many, 'one more'))).toBe(false);
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
    let board;
    beforeEach(() => {
        perm = base();
        board = baseBoard();
        window.showToast = vi.fn();
        window.api.permissionsGet = vi.fn(async () => structuredClone(perm));
        window.api.permissionsCommands = vi.fn(async () => structuredClone(board));
        window.api.permissionsUpdate = vi.fn(async () => ({ success: true }));
        window.api.permissionsSetMember = vi.fn(async () => ({ success: true }));
        window.api.permissionsRemoveMember = vi.fn(async () => ({ success: true }));
    });
    afterEach(() => {
        for (const k of API) delete window.api[k];
        delete window.api.relayListLegacyMembers;
        window.showToast = undefined;
    });

    it('사람: 레벨마다 번호와 사람을 보이고, 위 레벨은 아래 레벨의 명령을 모두 쓴다고 알린다', async () => {
        await openPeople();
        expect(window.api.permissionsGet).toHaveBeenCalledWith('g1');
        expect(within(level('Owner')).getByText('Aoba')).toBeInTheDocument();
        expect(within(level('Owner')).getByText('Lv 4')).toBeInTheDocument();
        expect(within(level('Owner')).getByText('Every command')).toBeInTheDocument();
        expect(within(level('Banned')).getByText('1004')).toBeInTheDocument();
        // 관리자에서 새로 쓰는 명령: 모든 인스턴스 기준 큰 칩 instance.create 하나 (mc-2 정지는 꼬리표 칩)
        expect(within(level('Admin')).getByText('Everything Lv 2 · User can use + 1')).toBeInTheDocument();
        // 금지 바로 위 레벨은 help·list와 거기 놓인 명령
        expect(within(level('Guest')).getByText('2 commands')).toBeInTheDocument();

        fireEvent.click(within(level('Admin')).getByRole('button', { name: 'Mint' }));
        pickOption(screen.getByLabelText('Level for Mint'), 'Lv 0 · Banned');
        await flush();
        expect(window.api.permissionsSetMember).toHaveBeenCalledWith('g1', '1002', -1);
        expect(window.api.permissionsGet).toHaveBeenCalledTimes(2);

        pickOption(screen.getByLabelText('Default level'), 'Lv 1 · Guest');
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', { defaultLevel: 0 });
    });

    it('사람: 기본 레벨인 디스코드 멤버를 펼쳐 볼 수 있고, 레벨에 사람을 더한다', async () => {
        await openPeople();
        const user = level('User');
        expect(within(user).queryByText('Newbie')).toBeNull();
        fireEvent.click(within(user).getByRole('button', { name: 'Show 1 on the default level' }));
        expect(within(user).getByRole('button', { name: 'Newbie' })).toBeInTheDocument();

        fireEvent.click(screen.getByRole('button', { name: 'Add person to Admin' }));
        pickOption(screen.getByLabelText('Pick a Discord member'), 'Newbie');
        await flush();
        expect(window.api.permissionsSetMember).toHaveBeenCalledWith('g1', '1003', 2);

        fireEvent.click(screen.getByRole('button', { name: 'Add person to Banned' }));
        fireEvent.change(screen.getByRole('textbox', { name: 'Discord user ID' }), {
            target: { value: '123456789012345678' },
        });
        fireEvent.click(screen.getByRole('button', { name: 'Add' }));
        await flush();
        expect(window.api.permissionsSetMember).toHaveBeenCalledWith('g1', '123456789012345678', -1);
    });

    it('디스코드 멤버를 못 받았으면 레벨 목록 아래에 그렇다고 알린다', async () => {
        await openPeople({ members: [] });
        expect(screen.getByText(/Couldn't load members/)).toBeInTheDocument();
        expect(screen.queryByRole('button', { name: /on the default level/ })).toBeNull();
    });

    it('데몬이 거절하면 알림을 띄우고 화면은 데몬 값 그대로 둔다', async () => {
        window.api.permissionsSetMember = vi.fn(async () => ({ error: 'x', error_code: 'owner_fixed' }));
        await openPeople();
        fireEvent.click(within(level('Admin')).getByRole('button', { name: 'Mint' }));
        pickOption(screen.getByLabelText('Level for Mint'), 'Lv 2 · User');
        await flush();
        expect(window.showToast).toHaveBeenCalledWith("The owner's level can't be changed.", 'error', 5000, undefined);
        expect(within(level('Admin')).getByRole('button', { name: 'Mint' })).toBeInTheDocument();
    });

    it('릴레이에 남은 부여값(legacy-members)을 골라 가져온다: 디스코드 멤버 목록에는 레벨이 없다', async () => {
        window.api.relayListLegacyMembers = vi.fn(async () => relayMembers);
        await openPeople({ members: relayMembers.map(({ level, ...m }) => m) });
        expect(window.api.relayListLegacyMembers).toHaveBeenCalledWith('g1');
        fireEvent.click(screen.getByRole('checkbox', { name: 'Newbie' }));
        fireEvent.click(screen.getByRole('button', { name: 'Import 1' }));
        await flush();
        expect(window.api.permissionsSetMember).toHaveBeenCalledWith('g1', '1003', 0);
    });

    it('새로고침하면 릴레이에 남은 부여값도 다시 읽는다', async () => {
        window.api.relayListLegacyMembers = vi.fn(async () => relayMembers);
        await openPeople({ members: relayMembers.map(({ level, ...m }) => m), onRefreshMembers: vi.fn() });
        expect(screen.getByRole('checkbox', { name: 'Newbie' })).toBeInTheDocument();

        fireEvent.click(screen.getByRole('checkbox', { name: 'Newbie' }));
        let release;
        window.api.relayListLegacyMembers.mockReturnValue(new Promise((r) => (release = r)));
        fireEvent.click(screen.getByRole('button', { name: /Refresh/ }));
        // 응답 전에는 옛 후보로 가져올 수 없다
        await flush();
        expect(screen.queryByRole('button', { name: /^Import \d/ })).toBeNull();
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
        await openPeople();
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

    it('명령 계단: 칩은 그 레벨 계단에 있고, 위 계단은 아래 계단의 명령 수를 이어받는다', async () => {
        await openCommands();
        expect(window.api.permissionsCommands).toHaveBeenCalledWith('g1', 'en');
        expect(within(step('Lv 1 · Guest')).getByRole('button', { name: /Help · list/ })).toBeInTheDocument();
        expect(within(step('Lv 1 · Guest')).getByRole('button', { name: /^Say$/ })).toBeInTheDocument();
        expect(within(step('Lv 2 · User')).getByRole('button', { name: /^Stop$/ })).toBeInTheDocument();
        expect(within(step('Lv 3 · Admin')).getByRole('button', { name: /Stop · Creative/ })).toBeInTheDocument();
        expect(within(step('Lv 3 · Admin')).getByText('3 from Lv 2 · User +')).toBeInTheDocument();
        expect(within(step('Lv 4 · Owner')).getByRole('button', { name: /Update/ })).not.toHaveAttribute('draggable');
        expect(screen.getByText('1 command(s) moved away from the preset')).toBeInTheDocument();
    });

    it('명령 계단: 칩을 누르고 레벨을 고르면 그 명령의 모든 인스턴스를 옮긴다', async () => {
        await openCommands();
        fireEvent.click(within(step('Lv 2 · User')).getByRole('button', { name: /^Stop$/ }));
        const edit = screen.getByRole('region', { name: 'Stop' });
        expect(within(edit).getByText('Applies to all 2 instances that have this command.')).toBeInTheDocument();
        fireEvent.click(within(edit).getByRole('button', { name: 'Lv 4 · Owner' }));
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', {
            commandLevels: { 'mc-1/stop': 3, 'mc-1/server.stop': null, 'mc-2/stop': 3, 'mc-2/server.stop': null },
        });
        expect(window.api.permissionsCommands).toHaveBeenCalledTimes(2);
    });

    it('명령 계단: 칩을 다른 계단으로 끌어 옮기고, 금지 계단에는 놓을 수 없다', async () => {
        await openCommands();
        const say = within(step('Lv 1 · Guest')).getByRole('button', { name: /^Say$/ });
        fireEvent.dragStart(say);
        fireEvent.dragOver(step('Lv 3 · Admin'));
        fireEvent.drop(step('Lv 3 · Admin'));
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', { commandLevels: { 'mc-1/say': 2, 'mc-2/say': 2 } });

        fireEvent.dragStart(within(step('Lv 1 · Guest')).getByRole('button', { name: /^Say$/ }));
        fireEvent.drop(step('Lv 0 · Banned'));
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledTimes(1);
    });

    it('명령 계단: 옮긴 칩은 옛 계단 자리에서 새 계단으로 미끄러진다', async () => {
        const steps = ['Lv 0 · Banned', 'Lv 1 · Guest', 'Lv 2 · User', 'Lv 3 · Admin', 'Lv 4 · Owner'];
        const rect = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function () {
            const left = steps.indexOf(this.closest?.('li')?.getAttribute('aria-label')) * 100;
            return { left, top: 0, width: 10, height: 10, right: left + 10, bottom: 10, x: left, y: 0 };
        });
        const animate = vi.fn();
        HTMLElement.prototype.animate = animate;
        try {
            await openCommands();
            const moved = baseBoard();
            moved.items = moved.items.map((i) => (i.name === 'say' ? { ...i, level: 2, overridden: true } : i));
            window.api.permissionsCommands.mockResolvedValue(moved);
            const say = within(step('Lv 1 · Guest')).getByRole('button', { name: /^Say$/ });
            fireEvent.dragStart(say);
            expect(say).toHaveClass('is-dragging');
            fireEvent.dragOver(step('Lv 3 · Admin'));
            fireEvent.drop(step('Lv 3 · Admin'));
            await flush();
            expect(within(step('Lv 3 · Admin')).getByRole('button', { name: /^Say$/ })).toBeInTheDocument();
            expect(animate).toHaveBeenCalledWith(
                [{ transform: 'translate(-200px, 0px)' }, { transform: 'none' }],
                expect.objectContaining({ duration: 260 }),
            );
        } finally {
            rect.mockRestore();
            delete HTMLElement.prototype.animate;
        }
    });

    it('명령 계단: 한 인스턴스 보기와 고정 레벨 명령', async () => {
        await openCommands();
        pickOption(screen.getByLabelText('View'), 'Creative');
        expect(within(step('Lv 3 · Admin')).getByRole('button', { name: /^Stop$/ })).toBeInTheDocument();
        fireEvent.click(within(step('Lv 4 · Owner')).getByRole('button', { name: /Update/ }));
        const edit = screen.getByRole('region', { name: 'Update' });
        expect(within(edit).getByText(/fixed and can't be moved/)).toBeInTheDocument();
        expect(within(edit).queryByRole('button', { name: /^Lv/ })).toBeNull();
    });

    it('인스턴스 메뉴의 계단은 그 인스턴스 명령과 서버 전체 명령만 보이고, 서버 전체 명령은 그렇다고 알린다', async () => {
        await openCommands({ instance: 'mc-2' });
        expect(screen.queryByLabelText('View')).toBeNull();
        expect(within(step('Lv 3 · Admin')).getByRole('button', { name: /^Stop$/ })).toBeInTheDocument();
        expect(screen.queryByRole('button', { name: /Stop · / })).toBeNull();
        fireEvent.click(within(step('Lv 3 · Admin')).getByRole('button', { name: /Create server/ }));
        expect(screen.getByText(/server-wide command/)).toBeInTheDocument();
    });

    it('명령별 레벨이 있으면 프리셋을 바꾸기 전에 다시 맞춘다고 묻고, 되돌리기도 같은 확인을 거친다', async () => {
        perm = base({ commandLevels: { 'mc-2/stop': 2 } });
        await openCommands();
        fireEvent.click(screen.getByRole('tab', { name: 'Strict' }));
        await flush();
        expect(window.api.permissionsUpdate).not.toHaveBeenCalled();
        expect(screen.getByRole('alert')).toHaveTextContent('1 command level(s) will be reset to the Strict preset.');
        fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
        fireEvent.click(screen.getByRole('button', { name: 'Reset to preset' }));
        expect(screen.getByRole('alert')).toHaveTextContent('1 command level(s) will be reset to the Default preset.');
        fireEvent.click(screen.getByRole('button', { name: 'Apply' }));
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', { preset: 'default' });
    });

    it('더한 레벨도 사람 레벨로 고를 수 있다', async () => {
        perm = base({
            ladder: [{ id: 3 }, { id: 2 }, { id: 10, name: 'VIP' }, { id: 1 }, { id: 0 }, { id: -1 }],
            members: { 1002: 10 },
        });
        await openPeople();
        expect(within(level('VIP')).getByRole('button', { name: 'Mint' })).toBeInTheDocument();
        fireEvent.click(within(level('VIP')).getByRole('button', { name: 'Mint' }));
        pickOption(screen.getByLabelText('Level for Mint'), 'Lv 4 · Admin');
        await flush();
        expect(window.api.permissionsSetMember).toHaveBeenCalledWith('g1', '1002', 2);
    });

    it('레벨 편집: 블록을 끌어 순서를 바꾸고, 더하고 지운 뒤 저장한다', async () => {
        perm = base({ ladder: [{ id: 3 }, { id: 2 }, { id: 10, name: 'VIP' }, { id: 1 }, { id: 0 }, { id: -1 }] });
        await openPeople();
        fireEvent.click(screen.getByRole('button', { name: /Edit levels/ }));
        const block = (name) => screen.getByRole('listitem', { name });
        expect(within(block('Owner')).queryByRole('button', { name: /Delete/ })).toBeNull();
        expect(block('Banned')).not.toHaveAttribute('draggable', 'true');

        fireEvent.dragStart(block('VIP'));
        fireEvent.dragOver(block('Admin'));
        fireEvent.drop(block('Admin'));
        fireEvent.click(screen.getByRole('button', { name: 'Delete User' }));
        expect(screen.getByText(/People on a deleted level move/)).toBeInTheDocument();
        fireEvent.change(screen.getByRole('textbox', { name: 'New level name' }), { target: { value: 'Mod' } });
        fireEvent.click(screen.getByRole('button', { name: 'Add' }));
        fireEvent.click(screen.getByRole('button', { name: 'Move Mod down' }));
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', {
            ladder: [{ id: 3 }, { id: 10, name: 'VIP' }, { name: 'Mod' }, { id: 2 }, { id: 0 }, { id: -1 }],
        });
        fireEvent.click(screen.getByRole('button', { name: /Back/ }));
        expect(screen.getByRole('tab', { name: 'Member levels' })).toBeInTheDocument();
    });

    it('디스코드 서버 → 인스턴스 메뉴에서 레벨별 명령과 멤버별 레벨을 따로 본다', async () => {
        render(
            <NodeView
                node={node}
                nodes={[node]}
                servers={servers}
                settings={{}}
                updateAllowed={vi.fn()}
                members={relayMembers}
            />,
        );
        expect(screen.queryByRole('tab', { name: 'Members' })).toBeNull();
        expect(screen.queryByRole('tab', { name: 'Permissions' })).toBeNull();
        fireEvent.click(screen.getByRole('button', { name: 'Survival permissions' }));
        await flush();
        expect(screen.getByRole('heading', { name: /Survival/ })).toBeInTheDocument();
        expect(screen.getByRole('tab', { name: 'Commands per level', selected: true })).toBeInTheDocument();
        fireEvent.click(screen.getByRole('tab', { name: 'Member levels' }));
        expect(screen.getByText(/apply to every instance on this Discord server/)).toBeInTheDocument();
        fireEvent.click(screen.getByRole('button', { name: /Instances/ }));
        expect(screen.getByRole('button', { name: 'Survival permissions' })).toBeInTheDocument();
    });
});

describe('sortByName', () => {
    it('sorts mixed Korean, English and numbered names like a dictionary', () => {
        const items = [{ n: 'user10' }, { n: '하늘' }, { n: 'Bob' }, { n: 'alice' }, { n: 'user2' }, { n: '가람' }];
        expect(L.sortByName(items, (i) => i.n).map((i) => i.n)).toEqual(['alice', 'Bob', 'user2', 'user10', '가람', '하늘']);
        expect(items[0].n).toBe('user10');
    });
});
