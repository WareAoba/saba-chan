import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CommandItem, GuildPermissions } from '../components/discord/levels';
import * as L from '../components/discord/levels';
import { NodeView } from '../components/discord/NodeView';
import { MembersPanel, OWNER_CONFIRM_MS, PermissionPanel } from '../components/discord/PermissionsTab';
import { pickOption } from './dropdown';

const API = [
    'permissionsGet',
    'permissionsCommands',
    'permissionsUpdate',
    'permissionsSetMember',
    'permissionsRemoveMember',
];
const node = { guildId: 'g1', guildName: 'Guild' };
const servers = [{ id: 'mc-1', name: 'Survival', module: 'minecraft' }];
const relayMembers = [
    { id: '1001', username: 'aoba', displayName: 'Aoba', level: 2 },
    { id: '1002', username: 'mint', displayName: 'Mint', level: 2 },
    { id: '1003', username: 'newbie', displayName: 'Newbie', level: 0 },
];
const base = (over?: Partial<GuildPermissions>): GuildPermissions => ({
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
const stair = (instance: string | null, name: string, title: string, level: number, over?: Record<string, unknown>) =>
    ({
        key: instance ? `${instance}/${name}` : name,
        keys: [instance ? `${instance}/${name}` : name],
        instance,
        name,
        title,
        kind: 'command',
        risk: 'caution',
        riskKey: instance ? `${instance}/${name}` : name,
        level,
        preset: level,
        overridden: false,
        fixed: false,
        ...over,
    }) as CommandItem;
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

function renderPanel(props: { instance?: string } = {}) {
    return render(<PermissionPanel node={node} {...props} />);
}

/** 인스턴스 메뉴의 레벨별 명령 화면 (처음 열리는 화면) */
async function openCommands(props?: { instance?: string }) {
    renderPanel(props);
    await flush();
}

/** 디스코드 서버의 멤버 탭 (멤버 레벨은 서버 하나에 한 번 정한다) */
async function openPeople(props = {}) {
    render(<MembersPanel node={node} members={relayMembers} {...props} />);
    await flush();
}

/**
 * 마우스로 끌기 (pointer 이벤트). Windows 웹뷰의 HTML 끌어 놓기에 기대지 않는다.
 * 놓는 자리는 elementFromPoint 가 정한다
 */
function pressAndMove(from: HTMLElement, to: HTMLElement) {
    // 끄는 요소는 끄는 동안만 손 아래에서 비켜 준다 (그 밖에는 손 아래가 그 요소 자신)
    document.elementFromPoint = () => (from.style.pointerEvents === 'none' ? to : from);
    fireEvent.pointerDown(from, { button: 0, clientX: 0, clientY: 0, pointerId: 1 });
    fireEvent.pointerMove(window, { clientX: 40, clientY: 40, pointerId: 1 });
}
const release = () => fireEvent.pointerUp(window, { clientX: 40, clientY: 40, pointerId: 1 });
function dragTo(from: HTMLElement, to: HTMLElement) {
    pressAndMove(from, to);
    release();
}
const step = (name: string) => screen.getByRole('listitem', { name });
const level = (name: string) => screen.getByRole('listitem', { name });

describe('권한 레벨 계산', () => {
    it('레벨 번호는 금지(Lv 0)부터 위로 센다: 더한 레벨이 있으면 밀린다', () => {
        expect(L.levelNumber(base(), 3)).toBe(4);
        expect(L.levelNumber(base(), -1)).toBe(0);
        const perm = base({
            ladder: [{ id: 3 }, { id: 10, name: 'VIP' }, { id: 2 }, { id: 1 }, { id: 0 }, { id: -1 }],
        });
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
        expect(chips.find((c) => c.name === 'say')!.items).toHaveLength(2);
        // 되묻기 단계는 칩마다 그 칩 인스턴스의 값 (바꾼 값이 없으면 원래 단계)
        const tiered = baseBoard();
        tiered.items[1] = { ...tiered.items[1], confirmRisk: 'danger' };
        expect(
            L.stairChips(tiered)
                .filter((c) => c.name === 'stop')
                .map((c) => c.confirmRisk),
        ).toEqual(['caution', 'danger']);
        // 같은 레벨인데 인스턴스마다 단계가 다르면 한쪽 값이 아니라 섞여 있다고 보인다
        const mixed = baseBoard();
        mixed.items[3] = { ...mixed.items[3], confirmRisk: 'danger' };
        const say = L.stairChips(mixed).find((c) => c.name === 'say');
        expect([say!.confirmRisk, say!.confirmMixed]).toEqual([null, true]);
        expect(L.stairChips(baseBoard()).find((c) => c.name === 'say')!.confirmMixed).toBe(false);
        // 대표 칩의 인스턴스 레벨이 서로 다르면 레벨도 섞여 있다 (꼬리표 칩은 한 인스턴스라 섞이지 않는다)
        expect(chips.filter((c) => c.name === 'stop').map((c) => c.levelMixed)).toEqual([true, false]);
        expect(chips.find((c) => c.name === 'say')!.levelMixed).toBe(false);
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
        expect(L.levelPatch(stop!.items, 2)).toEqual({
            'mc-1/stop': 2,
            'mc-1/server.stop': null,
            'mc-2/stop': 2,
            'mc-2/server.stop': null,
        });
        expect(L.levelPatch(stop!.items, 1)['mc-2/stop']).toBeNull();
        const update = baseBoard().items.find((i) => i.fixed);
        expect(L.levelPatch([update!], 1)).toEqual({});
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

    // 사용자 지정 2026-11-06: 모든 레벨의 이름을 바꾼다 (비우면 로케일 이름). 소유자·게스트·금지는 지울 수 없다
    it('사다리 편집: 기본 레벨 이름도 보내고, 게스트는 지우지 않는다', () => {
        const ladder = L.ladderOf(base());
        const named = L.renameLevel(L.renameLevel(ladder, 0, ' 방장 '), 3, '  ');
        expect(L.ladderPatch(named)).toEqual([{ id: 3, name: '방장' }, { id: 2 }, { id: 1 }, { id: 0 }, { id: -1 }]);
        expect(L.ladderValid(named)).toBe(true);
        expect(L.ladderValid(L.renameLevel(ladder, 1, 'x'.repeat(33)))).toBe(false);
        expect(ladder.map((_, i) => L.removable(ladder, i))).toEqual([false, true, true, false, false]);
        expect(L.removeLevel(ladder, 3)).toBe(ladder);
    });

    it('조작 가능 줄: 시작·정지·재시작·백업 만들기의 대표 레벨 아래에 긋고, 따로 정한 명령은 옮기지 않는다', () => {
        const board = {
            items: [
                stair('mc-1', 'start', 'Start', 2, { controllers: true, preset: 2 }),
                stair('mc-2', 'start', 'Start', 2, { controllers: true, preset: 2 }),
                stair('mc-2', 'stop', 'Stop', 1, { controllers: true, preset: 2, overridden: true }),
                stair('mc-1', 'say', 'Say', 0),
            ],
        };
        const line = L.controllerLine(board);
        expect(line?.level).toBe(2);
        expect(line?.apart).toBe(1);
        expect(L.levelPatch(line?.followers || [], 0)).toEqual({ 'mc-1/start': 0, 'mc-2/start': 0 });
        expect(L.controllerLine({ items: [stair('mc-1', 'say', 'Say', 0)] })).toBeNull();

        const saved = L.ladderOf(base());
        // 줄은 금지 칸과 아직 저장하지 않은 새 칸에는 놓지 않는다
        const draft = L.addLevel(saved, 'Mod');
        expect(L.lineDrop(draft, 5)).toBe(4);
        expect(L.lineDrop(draft, 1)).toBe(0);
        expect(L.lineStep(draft, 0, 1)).toBe(2);
        expect(L.lineStep(draft, 4, 1)).toBeNull();
        expect(L.lineStep(draft, 0, -1)).toBeNull();
        // 줄이 있던 칸을 지우면 그 위로 처음 남은 칸 (데몬이 명령 레벨을 위로 옮긴다)
        expect(L.lineAt(saved, L.removeLevel(saved, 1), 2)).toBe(0);
        expect(L.lineAt(saved, saved, 1)).toBe(2);
    });

    // 사용자 지정 2026-11-07: 사람은 두고 두 레벨이 그 레벨부터 가진 명령만 바꾼다
    it('권한 맞바꾸기: 두 레벨의 명령만 바꾸고, 고정 명령·지운 레벨은 그대로, 줄은 명령을 따라간다', () => {
        const board = baseBoard();
        const ladder = L.ladderOf(base());
        // 항목 순서: mc-1 stop(1) · mc-2 stop(2) · say(0) · say(0) · instance.create(2) · update.apply(3, 고정)
        expect(L.draftLevels(board, ladder, [[1, 2]])).toEqual([2, 1, 0, 0, 1, 3]);
        expect(
            L.draftLevels(board, ladder, [
                [1, 2],
                [2, 1],
            ]),
        ).toEqual([1, 2, 0, 0, 2, 3]);
        expect(L.draftLevels(board, ladder, [[3, 0]])).toEqual([1, 2, 3, 3, 2, 3]);
        // 초안에서 지운 레벨이 걸린 맞바꾸기는 뺀다 (데몬이 지운 레벨의 명령을 위로 옮긴다)
        expect(L.draftLevels(board, L.removeLevel(ladder, 2), [[1, 2]])).toEqual([1, 2, 0, 0, 2, 3]);
        expect(L.boardPatch(board, L.draftLevels(board, ladder, [[1, 2]]))).toEqual({
            'mc-1/stop': 2,
            'mc-1/server.stop': null,
            'mc-2/stop': null,
            'mc-2/server.stop': null,
            'instance.create': 1,
        });
        expect(L.boardPatch(board, L.draftLevels(board, ladder, []))).toEqual({});
        expect(L.swapLine(2, 1, 2)).toBe(1);
        expect(L.swapLine(0, 1, 2)).toBe(0);
        // 줄을 따르는 명령은 줄 자리로 (맞바꾸기 뒤에 줄을 옮겨도)
        const line = { followers: [board.items[4]] };
        expect(L.draftLevels(board, ladder, [[1, 2]], line, 3)).toEqual([2, 1, 0, 0, 3, 3]);
        expect(ladder.map(L.swappable)).toEqual([true, true, true, true, false]);
        expect(L.swappable({ key: 'new-1', name: 'Mod' })).toBe(false);
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
    let perm: GuildPermissions;
    let board: ReturnType<typeof baseBoard>;
    beforeEach(() => {
        perm = base();
        board = baseBoard();
        window.showToast = vi.fn();
        window.api.permissionsGet = vi.fn(async () => structuredClone(perm));
        window.api.permissionsCommands = vi.fn(async () => structuredClone(board));
        window.api.permissionsUpdate = vi.fn(async () => ({ success: true }));
        window.api.permissionsSetMember = vi.fn(async () => ({ success: true }));
        window.api.permissionsRemoveMember = vi.fn(async () => ({ success: true }));
        window.api.discordSettingsSet = vi.fn(async () => ({ success: true }));
    });
    afterEach(() => {
        for (const k of API) delete (window.api as unknown as Record<string, unknown>)[k];
        delete (window.api as Partial<typeof window.api>).discordSettingsSet;
        delete (window.api as Partial<typeof window.api>).relayListLegacyMembers;
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

    it('사람: 칩을 다른 레벨로 끌어 옮기고, 소유자 레벨에는 놓을 수 없다', async () => {
        await openPeople();
        const mint = within(level('Admin')).getByRole('button', { name: 'Mint' });
        // HTML 끌어 놓기(draggable)는 Windows 웹뷰에서 막힐 수 있어 쓰지 않는다
        expect(mint).not.toHaveAttribute('draggable');
        pressAndMove(mint, level('Guest'));
        expect(mint).toHaveClass('is-dragging');
        expect(level('Guest')).toHaveClass('is-over');
        release();
        await flush();
        expect(window.api.permissionsSetMember).toHaveBeenCalledWith('g1', '1002', 0);

        // 소유자 레벨과 원래 레벨에는 놓아도 바꾸지 않는다
        dragTo(within(level('Admin')).getByRole('button', { name: 'Mint' }), level('Owner'));
        dragTo(within(level('Admin')).getByRole('button', { name: 'Mint' }), level('Admin'));
        await flush();
        expect(window.api.permissionsSetMember).toHaveBeenCalledTimes(1);
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
        let release!: (members: typeof relayMembers) => void;
        vi.mocked(window.api.relayListLegacyMembers).mockReturnValue(
            new Promise((r) => (release = r as typeof release)),
        );
        fireEvent.click(screen.getByRole('button', { name: /Refresh/ }));
        // 응답 전에는 옛 후보로 가져올 수 없다
        await flush();
        expect(screen.queryByRole('button', { name: /^Import \d/ })).toBeNull();
        await act(async () => release(relayMembers));
        expect(screen.getByRole('checkbox', { name: 'Newbie' })).not.toBeChecked();

        vi.mocked(window.api.relayListLegacyMembers).mockResolvedValue([]);
        fireEvent.click(screen.getByRole('button', { name: /Refresh/ }));
        await flush();
        expect(window.api.relayListLegacyMembers).toHaveBeenCalledTimes(3);
        expect(screen.queryByRole('checkbox', { name: 'Newbie' })).toBeNull();
    });

    it('소유자 불일치를 알리고, 고르면 소유자를 바꾼다', async () => {
        perm = base({ relayOwnerDiscordId: '1002' });
        await openPeople();
        expect(screen.getByRole('alert')).toHaveTextContent(
            'Server-chan reported a different owner (Mint). The owner here stays Aoba.',
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

    it('명령 목록: 명령마다 누구부터 쓸 수 있는지 보이고, 요약은 레벨마다 쓸 수 있는 명령 수(아래 레벨을 이어받는다)', async () => {
        await openCommands();
        expect(window.api.permissionsCommands).toHaveBeenCalledWith('g1', 'en');
        // 인스턴스마다 레벨이 다르면 대표 레벨이 아니라 섞여 있다고 보인다
        expect(screen.getByLabelText('Stop: who can use it')).toHaveTextContent('Varies by instance');
        expect(screen.getByLabelText('Stop · Creative: who can use it')).toHaveTextContent('Admin and up');
        expect(screen.getByLabelText('Say: who can use it')).toHaveTextContent('Guest and up');
        expect(screen.getByLabelText('Update: who can use it')).toHaveTextContent('Owner only');
        expect(screen.getByText('Guest 2 · User 3 · Admin 4 · Owner 5')).toBeInTheDocument();
        expect(within(screen.getByRole('region', { name: 'Management' })).getByText('Create server')).toBeInTheDocument();
        expect(screen.getByText('1 command(s) moved away from the preset')).toBeInTheDocument();
        expect(screen.getAllByText(/2 instance\(s\)/).length).toBeGreaterThan(0);
    });

    it('명령 목록: 레벨을 고르면 그 명령의 모든 인스턴스를 옮긴다', async () => {
        await openCommands();
        pickOption(screen.getByLabelText('Stop: who can use it'), 'Owner only');
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', {
            commandLevels: { 'mc-1/stop': 3, 'mc-1/server.stop': null, 'mc-2/stop': 3, 'mc-2/server.stop': null },
        });
        expect(window.api.permissionsCommands).toHaveBeenCalledTimes(2);
    });

    it('명령 목록: 레벨이 섞인 명령은 많은 쪽 레벨을 골라도 모든 인스턴스를 맞춘다', async () => {
        await openCommands();
        pickOption(screen.getByLabelText('Stop: who can use it'), 'User and up');
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', {
            commandLevels: { 'mc-1/stop': null, 'mc-1/server.stop': null, 'mc-2/stop': null, 'mc-2/server.stop': null },
        });
    });

    it('명령 목록: 그 줄에서 디스코드 되묻기 단계를 바꾸고, 원래 단계를 고르면 지운다', async () => {
        await openCommands();
        board.items = board.items.map((i) => (i.name === 'say' ? { ...i, confirmRisk: 'danger' } : i));
        pickOption(screen.getByLabelText('Say: Discord confirmation'), /^Danger/);
        await flush();
        expect(window.api.discordSettingsSet).toHaveBeenCalledWith('g1', {
            commandRisks: { 'mc-1/say': 'danger', 'mc-2/say': 'danger' },
        });
        expect(window.api.permissionsUpdate).not.toHaveBeenCalled();
        expect(screen.getByLabelText('Say: Discord confirmation')).toHaveTextContent('Danger (Default: Safe)');
        pickOption(screen.getByLabelText('Say: Discord confirmation'), 'Safe');
        await flush();
        expect(window.api.discordSettingsSet).toHaveBeenLastCalledWith('g1', {
            commandRisks: { 'mc-1/say': null, 'mc-2/say': null },
        });
    });

    it('명령 목록: 찾기는 이름으로 거르고, 한 인스턴스 보기와 고정 레벨 명령', async () => {
        await openCommands();
        fireEvent.change(screen.getByLabelText('Find a command'), { target: { value: 'sto' } });
        expect(screen.queryByLabelText('Say: who can use it')).toBeNull();
        expect(screen.getByLabelText('Stop: who can use it')).toBeInTheDocument();
        fireEvent.change(screen.getByLabelText('Find a command'), { target: { value: 'zzz' } });
        expect(screen.getByText('No matching command.')).toBeInTheDocument();
        fireEvent.change(screen.getByLabelText('Find a command'), { target: { value: '' } });
        pickOption(screen.getByLabelText('View'), 'Creative');
        expect(screen.getByLabelText('Stop: who can use it')).toHaveTextContent('Admin and up');
        expect(screen.getByLabelText('Update: who can use it')).toBeDisabled();
        expect(screen.getByText(/affects the whole PC/)).toBeInTheDocument();
    });

    it('인스턴스 메뉴의 목록은 그 인스턴스 명령과 서버 전체 명령만 보이고, 서버 전체 명령은 그렇다고 알린다', async () => {
        await openCommands({ instance: 'mc-2' });
        expect(screen.queryByLabelText('View')).toBeNull();
        expect(screen.getByLabelText('Stop: who can use it')).toHaveTextContent('Admin and up');
        expect(screen.queryByLabelText(/Stop · /)).toBeNull();
        expect(screen.getAllByText(/Server-wide/).length).toBeGreaterThan(0);
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
        const block = (name: string) => screen.getByRole('listitem', { name });
        expect(within(block('Owner')).queryByRole('button', { name: /Delete/ })).toBeNull();
        expect(block('Banned')).not.toHaveAttribute('data-draggable');
        expect(block('VIP')).toHaveAttribute('data-draggable', 'true');

        dragTo(block('VIP'), block('Admin'));
        fireEvent.click(screen.getByRole('button', { name: 'Delete User' }));
        expect(screen.getByText(/People on a deleted level move/)).toBeInTheDocument();
        fireEvent.change(screen.getByRole('textbox', { name: 'New level name' }), { target: { value: 'Mod' } });
        fireEvent.click(screen.getByRole('button', { name: 'Add' }));
        fireEvent.click(screen.getByRole('button', { name: 'Move Mod down' }));
        // 사용자 11-06 #9: 저장은 화면 아래에 붙는 막대에서
        expect(document.querySelector('.dc-savebar.dirty')).not.toBeNull();
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', {
            ladder: [{ id: 3 }, { id: 10, name: 'VIP' }, { name: 'Mod' }, { id: 2 }, { id: 0 }, { id: -1 }],
        });
        fireEvent.click(screen.getByRole('button', { name: /Back/ }));
        expect(screen.getByRole('button', { name: /Edit levels/ })).toBeInTheDocument();
    });

    it('레벨 편집: 기본 레벨 이름을 바꾸고, 게스트는 지울 수 없다', async () => {
        await openPeople();
        fireEvent.click(screen.getByRole('button', { name: /Edit levels/ }));
        expect(screen.getByRole('button', { name: 'Delete Guest' })).toBeDisabled();
        expect(screen.getByRole('button', { name: 'Delete User' })).toBeEnabled();
        // 이름을 누르면 고칠 칸이 열린다 (비우면 원래 이름)
        fireEvent.click(screen.getByRole('button', { name: 'Rename Admin' }));
        const name = screen.getByRole('textbox', { name: 'Level name' });
        expect(name).toHaveAttribute('placeholder', 'Admin');
        fireEvent.change(name, { target: { value: 'Staff' } });
        fireEvent.blur(name);
        expect(screen.getByRole('listitem', { name: 'Staff' })).toBeInTheDocument();
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', {
            ladder: [{ id: 3 }, { id: 2, name: 'Staff' }, { id: 1 }, { id: 0 }, { id: -1 }],
        });
    });

    // 사용자 지정 2026-11-06: 사다리에 조작 가능 줄을 긋고 끌어서 네 명령을 한꺼번에 옮긴다
    it('레벨 편집: 조작 가능 줄을 끌어 옮기면 줄을 따르는 명령만 저장한다', async () => {
        board.items.push(
            stair('mc-1', 'start', 'Start', 2, { controllers: true, preset: 2 }),
            stair('mc-2', 'start', 'Start', 2, { controllers: true, preset: 2 }),
            stair('mc-2', 'restart', 'Restart', 3, { controllers: true, preset: 2, overridden: true }),
        );
        await openPeople();
        fireEvent.click(screen.getByRole('button', { name: /Edit levels/ }));
        const line = screen.getByRole('listitem', { name: 'Operators line' });
        expect(line).toHaveTextContent('Operators · Admin and up');
        expect(line).toHaveTextContent('1 command(s) set separately stay as they are');
        // 줄은 관리자 칸 바로 아래
        const items = screen.getAllByRole('listitem');
        expect(items.indexOf(line)).toBe(items.indexOf(screen.getByRole('listitem', { name: 'Admin' })) + 1);

        dragTo(line, screen.getByRole('listitem', { name: 'Banned' }));
        expect(screen.getByRole('listitem', { name: 'Operators line' })).toHaveTextContent('Guest and up');
        fireEvent.click(screen.getByRole('button', { name: 'Move the operators line up' }));
        fireEvent.click(screen.getByRole('button', { name: 'Move the operators line up' }));
        fireEvent.click(screen.getByRole('button', { name: 'Move the operators line up' }));
        expect(screen.getByRole('listitem', { name: 'Operators line' })).toHaveTextContent('Owner only');
        expect(screen.getByRole('button', { name: 'Move the operators line up' })).toBeDisabled();
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', {
            commandLevels: { 'mc-1/start': 3, 'mc-2/start': 3 },
        });
    });

    it('레벨 편집: 권한 맞바꾸기를 누르고 두 레벨을 고르면 사람은 두고 명령만 바꿔 저장한다', async () => {
        await openPeople();
        fireEvent.click(screen.getByRole('button', { name: /Edit levels/ }));
        // 칸마다 그 레벨부터 쓰는 명령 칩
        expect(within(level('User')).getByText('Stop')).toBeInTheDocument();
        expect(within(level('Admin')).getByText('Create server')).toBeInTheDocument();
        expect(within(level('Owner')).getByText('Update')).toBeInTheDocument();
        expect(screen.queryByRole('button', { name: 'Pick User' })).toBeNull();

        fireEvent.click(screen.getByRole('button', { name: 'Swap permissions' }));
        expect(screen.getByText('Pick two levels to swap (0/2)')).toBeInTheDocument();
        expect(screen.queryByRole('button', { name: 'Pick Banned' })).toBeNull();
        fireEvent.click(screen.getByRole('button', { name: 'Pick User' }));
        expect(screen.getByText(/User ⇄ \? · pick one more level \(1\/2\)/)).toBeInTheDocument();
        fireEvent.click(screen.getByRole('button', { name: 'Pick Admin' }));
        expect(within(level('Admin')).getByText('Stop')).toBeInTheDocument();
        expect(within(level('User')).getByText('Create server')).toBeInTheDocument();
        expect(screen.getByText(/User ⇄ Admin · People stay where they are/)).toBeInTheDocument();
        expect(screen.queryByRole('button', { name: 'Pick User' })).toBeNull();

        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', {
            commandLevels: {
                'mc-1/stop': 2,
                'mc-1/server.stop': null,
                'mc-2/stop': null,
                'mc-2/server.stop': null,
                'instance.create': 1,
            },
        });
    });

    it('멤버 레벨은 디스코드 서버의 멤버 탭에서 정하고, 인스턴스 메뉴는 레벨별 명령만 보인다', async () => {
        render(
            <NodeView
                node={node}
                nodes={[node]}
                servers={servers as never}
                settings={{}}
                updateAllowed={vi.fn()}
                updateStartNotice={undefined as never}
                updateFullNotice={undefined as never}
                members={relayMembers}
            />,
        );
        fireEvent.click(screen.getByRole('tab', { name: 'Members' }));
        await flush();
        expect(screen.getByText(/apply to every instance on this Discord server/)).toBeInTheDocument();
        expect(within(level('Admin')).getByRole('button', { name: 'Mint' })).toBeInTheDocument();

        fireEvent.click(screen.getByRole('tab', { name: 'Instances' }));
        fireEvent.click(screen.getByRole('button', { name: 'Survival permissions' }));
        await flush();
        expect(screen.getByRole('heading', { name: /Survival/ })).toBeInTheDocument();
        expect(screen.queryByRole('tab', { name: 'Member levels' })).toBeNull();
        expect(screen.queryByRole('button', { name: 'Mint' })).toBeNull();
        expect(screen.getByLabelText('Stop: who can use it')).toBeInTheDocument();
        fireEvent.click(screen.getByRole('button', { name: /Instances/ }));
        expect(screen.getByRole('button', { name: 'Survival permissions' })).toBeInTheDocument();
    });
});

describe('sortByName', () => {
    it('sorts mixed Korean, English and numbered names like a dictionary', () => {
        const items = [{ n: 'user10' }, { n: '하늘' }, { n: 'Bob' }, { n: 'alice' }, { n: 'user2' }, { n: '가람' }];
        expect(L.sortByName(items, (i) => i.n).map((i) => i.n)).toEqual([
            'alice',
            'Bob',
            'user2',
            'user10',
            '가람',
            '하늘',
        ]);
        expect(items[0].n).toBe('user10');
    });
});
