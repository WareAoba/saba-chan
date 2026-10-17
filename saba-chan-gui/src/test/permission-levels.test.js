import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as L from '../components/discord/levels';
import { NodeView } from '../components/discord/NodeView';
import { PermissionPanel } from '../components/discord/PermissionsTab';

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

        fireEvent.change(screen.getByLabelText('Level for Mint'), { target: { value: '-1' } });
        await flush();
        expect(window.api.permissionsSetMember).toHaveBeenCalledWith('g1', '1002', -1);
        expect(window.api.permissionsGet).toHaveBeenCalledTimes(2);

        fireEvent.change(screen.getByLabelText('Default level'), { target: { value: '0' } });
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', { defaultLevel: 0 });

        fireEvent.click(screen.getByRole('tab', { name: 'Strict' }));
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', { preset: 'strict' });
    });

    it('데몬이 거절하면 알림을 띄우고 화면은 데몬 값 그대로 둔다', async () => {
        window.api.permissionsSetMember = vi.fn(async () => ({ error: 'x', error_code: 'owner_fixed' }));
        renderPanel();
        await flush();
        fireEvent.change(screen.getByLabelText('Level for Mint'), { target: { value: '1' } });
        await flush();
        expect(window.showToast).toHaveBeenCalledWith("The owner's level can't be changed.", 'error', 5000, undefined);
        expect(screen.getByLabelText('Level for Mint')).toHaveValue('2');
    });

    it('릴레이에 남은 부여값을 골라 가져온다', async () => {
        renderPanel();
        await flush();
        fireEvent.click(screen.getByRole('checkbox', { name: 'Newbie' }));
        fireEvent.click(screen.getByRole('button', { name: 'Import 1' }));
        await flush();
        expect(window.api.permissionsSetMember).toHaveBeenCalledWith('g1', '1003', 0);
    });

    it('소유자 불일치를 알리고, 고르면 소유자를 바꾼다', async () => {
        perm = base({ relayOwnerDiscordId: '1002' });
        renderPanel();
        await flush();
        expect(screen.getByRole('alert')).toHaveTextContent(
            'The relay reported a different owner (Mint). The owner here stays Aoba.',
        );
        fireEvent.click(screen.getByRole('button', { name: 'Use this owner' }));
        await flush();
        expect(window.api.permissionsUpdate).not.toHaveBeenCalled(); // 한 번 더 눌러야 바꾼다
        fireEvent.click(screen.getByRole('button', { name: 'Really make Mint (1002) the owner' }));
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', { ownerDiscordId: '1002' });

        fireEvent.click(screen.getByRole('button', { name: 'Change' }));
        fireEvent.change(screen.getByRole('textbox', { name: 'Owner' }), { target: { value: '123456789012345678' } });
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', { ownerDiscordId: '123456789012345678' });
    });

    it('명령 탭: 기본은 프리셋을 따르고, 고르면 그 명령만 덮어쓰며, 프리셋으로 되돌리면 지운다', async () => {
        perm = base({ commandLevels: { 'mc-1/say': 3 } });
        renderPanel('commands');
        await flush();
        const stop = screen.getByLabelText('Level needed for Stop');
        expect(stop).toHaveValue('');
        expect(within(stop).getByRole('option', { name: 'Preset (User)' })).toBeInTheDocument();
        expect(screen.getByLabelText('Level needed for Say')).toHaveValue('3');

        fireEvent.change(stop, { target: { value: '2' } });
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', { commandLevels: { 'mc-1/stop': 2 } });
        fireEvent.change(screen.getByLabelText('Level needed for Say'), { target: { value: '' } });
        await flush();
        expect(window.api.permissionsUpdate).toHaveBeenCalledWith('g1', { commandLevels: { 'mc-1/say': null } });
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
