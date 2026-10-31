import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { assignPatch, kindOf, ModerationTab, modErrorText } from '../components/discord/ModerationTab';
import { NodeView } from '../components/discord/NodeView';
import { pickOption } from './dropdown';

const node = { guildId: 'g1', guildName: 'My Guild' };
const assigned = {
    guildId: 'g1',
    chatbot: ['111'],
    music: '222',
    notify: null,
    gameChat: null,
    board: null,
    botAvailable: true,
    channels: [
        { id: '111', name: 'chat', category: null },
        { id: '222', name: 'music', category: null },
        { id: '333', name: 'news', category: null },
    ],
};
const channel = (id, name, extra = {}) => ({
    id,
    name,
    category: null,
    topic: null,
    slowmode: 0,
    locked: null,
    autoClear: null,
    ...extra,
});
const moderation = {
    guildId: 'g1',
    botAvailable: true,
    can: { manageMessages: true, manageChannels: true, manageRoles: true, moderateMembers: true },
    channels: [
        channel('111', 'chat', { slowmode: 10, autoClear: 1440 }),
        channel('222', 'music'),
        channel('333', 'news', { locked: { until: null } }),
    ],
    mutes: [
        { userId: 'u2', name: 'Forever', forever: true, until: new Date(Date.now() + 864e5).toISOString() },
        { userId: 'u1', name: 'Soon', forever: false, until: new Date(Date.now() + 42 * 60000).toISOString() },
    ],
};
const members = [
    { id: 'u1', displayName: 'Soon' },
    { id: 'u3', displayName: 'Spammer' },
];

describe('배정 바꾸기', () => {
    const saved = { chatbot: ['111'], music: '222', notify: null, gameChat: null, board: null };

    it('한 채널은 한 가지 일 — 다른 일을 고르면 원래 일에서 빠지고, 한 채널짜리 일은 이 채널로 옮긴다', () => {
        expect(kindOf(saved, '222')).toBe('music');
        expect(assignPatch(saved, '111', 'notify')).toEqual({ chatbot: [], notify: '111' });
        expect(assignPatch(saved, '333', 'music')).toEqual({ music: '333' });
        expect(assignPatch(saved, '222', 'chatbot')).toEqual({ chatbot: ['111', '222'], music: null });
        expect(assignPatch(saved, '111', null)).toEqual({ chatbot: [] });
    });

    it('릴레이 오류는 문구로 — 빠진 봇 권한은 이름으로, 이름 바꾸기 한도는 분으로', () => {
        const t = (k, o) => (o ? `${k}:${JSON.stringify(o)}` : k);
        expect(modErrorText({ error: 'BOT_MISSING_PERMISSION', permission: 'ManageRoles' }, t)).toContain(
            'discord.mod.perm_ManageRoles',
        );
        expect(modErrorText({ error: 'RENAME_LIMIT', retryAfter: 125 }, t)).toBe(
            'discord.mod.err_RENAME_LIMIT:{"minutes":3}',
        );
        expect(modErrorText({ error: 'WHATEVER' }, t)).toBe('discord.mod.err_generic');
    });
});

describe('ModerationTab', () => {
    beforeEach(() => {
        window.api.relayNodeModeration = vi.fn().mockResolvedValue(moderation);
        window.api.relayNodeChannels = vi.fn().mockResolvedValue(assigned);
        window.api.relayNodeChannelsSave = vi.fn().mockResolvedValue({ ...assigned, chatbot: [], notify: '111' });
        window.api.relayModChannel = vi.fn(async (_g, id, patch) => ({ ...channel(id, 'chat'), ...patch }));
        window.api.relayModClear = vi.fn().mockResolvedValue({ deleted: 87, kept: 2, olderLeft: false });
        window.api.relayModMute = vi.fn().mockResolvedValue({ result: 'muted' });
    });
    afterEach(() => {
        for (const k of [
            'relayNodeModeration',
            'relayNodeChannels',
            'relayNodeChannelsSave',
            'relayModChannel',
            'relayModClear',
            'relayModMute',
        ]) {
            delete window.api[k];
        }
    });

    it('채널마다 배정과 걸린 것을 배지로 보이고, 고른 채널의 배정을 칩으로 바꾼다', async () => {
        render(<ModerationTab node={node} members={members} />);
        const list = await screen.findByRole('listbox', { name: 'Channels' });
        const chat = within(list).getByRole('option', { name: /#chat/ });
        expect(chat.textContent).toMatch(/Chat.*Slow 10s.*Keep 1d/);
        expect(within(list).getByRole('option', { name: /#news/ }).textContent).toMatch(/Locked/);
        fireEvent.click(screen.getByRole('button', { name: 'Alerts' }));
        await waitFor(() =>
            expect(window.api.relayNodeChannelsSave).toHaveBeenCalledWith('g1', { chatbot: [], notify: '111' }),
        );
    });

    it('노래 채널로 바꾸면 메시지 삭제 경고를 한 번 더 확인받는다', async () => {
        render(<ModerationTab node={node} members={members} />);
        fireEvent.click(within(await screen.findByRole('listbox')).getByRole('option', { name: /#news/ }));
        fireEvent.click(screen.getByRole('button', { name: /^Singing Saba-chan/ }));
        expect(window.api.relayNodeChannelsSave).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole('button', { name: /Save and delete messages/ }));
        await waitFor(() => expect(window.api.relayNodeChannelsSave).toHaveBeenCalledWith('g1', { music: '333' }));
    });

    it('이름·주제를 저장하고, 슬로우를 고르면 바로 바꾼다', async () => {
        render(<ModerationTab node={node} members={members} />);
        const name = await screen.findByLabelText('Name');
        fireEvent.change(name, { target: { value: 'lobby' } });
        fireEvent.change(screen.getByLabelText('Topic'), { target: { value: 'hi' } });
        fireEvent.click(screen.getByRole('button', { name: /^Save$/ }));
        await waitFor(() =>
            expect(window.api.relayModChannel).toHaveBeenCalledWith('g1', '111', { name: 'lobby', topic: 'hi' }),
        );
        pickOption(screen.getByLabelText('Slow mode'), '30s');
        await waitFor(() => expect(window.api.relayModChannel).toHaveBeenCalledWith('g1', '111', { slowmode: 30 }));
    });

    it('청소는 확인 창을 거쳐 지우고 결과를 알린다', async () => {
        render(<ModerationTab node={node} members={members} />);
        fireEvent.click(await screen.findByRole('button', { name: /Clear…/ }));
        expect(window.api.relayModClear).not.toHaveBeenCalled();
        const confirm = screen.getByRole('alertdialog');
        expect(confirm.textContent).toMatch(/Delete up to 100 messages in #chat/);
        fireEvent.click(within(confirm).getByRole('button', { name: 'Delete' }));
        await waitFor(() => expect(window.api.relayModClear).toHaveBeenCalledWith('g1', '111', { count: 100 }));
        expect(await screen.findByText(/Deleted 87 messages/)).toBeTruthy();
    });

    it('멤버 화면은 남은 시간 순으로 음소거된 사람을 보이고, 새로 음소거·해제한다', async () => {
        render(<ModerationTab node={node} members={members} />);
        fireEvent.click(await screen.findByRole('tab', { name: /Members/ }));
        const rows = screen.getAllByRole('listitem');
        expect(rows[0].textContent).toMatch(/Soon.*left/);
        expect(rows[1].textContent).toMatch(/Forever.*renews/);
        pickOption(screen.getByLabelText('Member'), 'Spammer');
        pickOption(screen.getByLabelText('Duration'), 'Forever');
        fireEvent.change(screen.getByLabelText('Reason (audit log)'), { target: { value: 'spam' } });
        fireEvent.click(screen.getByRole('button', { name: /^Mute$/ }));
        await waitFor(() =>
            expect(window.api.relayModMute).toHaveBeenCalledWith('g1', 'u3', { duration: 'forever', reason: 'spam' }),
        );
        fireEvent.click(within(rows[0]).getByRole('button', { name: 'Unmute' }));
        await waitFor(() => expect(window.api.relayModMute).toHaveBeenCalledWith('g1', 'u1', { duration: 'off' }));
    });

    it('목록에 없는 채널(삭제·권한 없음)에 남은 배정도 해제할 수 있다', async () => {
        window.api.relayNodeChannels = vi.fn().mockResolvedValue({ ...assigned, chatbot: ['111', '999'] });
        render(<ModerationTab node={node} members={members} />);
        const stale = await screen.findByRole('list', { name: 'Assignments to missing channels' });
        expect(stale.textContent).toMatch(/Chat.*999/);
        fireEvent.click(within(stale).getByRole('button', { name: 'Unassign' }));
        await waitFor(() => expect(window.api.relayNodeChannelsSave).toHaveBeenCalledWith('g1', { chatbot: ['111'] }));
    });

    it('사바쨩이 서버에 없어도 남은 배정은 해제할 수 있다', async () => {
        window.api.relayNodeModeration = vi
            .fn()
            .mockResolvedValue({ ...moderation, botAvailable: false, channels: [] });
        render(<ModerationTab node={node} />);
        const stale = await screen.findByRole('list', { name: 'Assignments to missing channels' });
        expect(within(stale).getAllByRole('button', { name: 'Unassign' })).toHaveLength(2);
    });

    it('사바쨩이 서버에 없으면 알린다', async () => {
        window.api.relayNodeModeration = vi
            .fn()
            .mockResolvedValue({ ...moderation, botAvailable: false, channels: [] });
        render(<ModerationTab node={node} />);
        expect(await screen.findByText(/isn't in this server/)).toBeTruthy();
    });
});

describe('NodeView', () => {
    it('채널 탭 대신 서버 관리 탭이 있고, 인스턴스 목록은 모듈 아이콘을 보인다', () => {
        window.api.relayNodeModeration = vi.fn();
        window.api.relayNodeChannels = vi.fn();
        window.api.relayNodeChannelsSave = vi.fn();
        const { container } = render(
            <NodeView
                node={node}
                nodes={[node]}
                servers={[
                    { id: 's1', name: 'Survival', module: 'minecraft' },
                    { id: 's2', name: 'Terra', module: 'terraria' },
                ]}
                modules={[{ name: 'minecraft', icon: 'data:image/png;base64,AA' }]}
                settings={{}}
                updateAllowed={() => {}}
                updateStartNotice={() => {}}
            />,
        );
        expect(screen.getByRole('tab', { name: /Server tools/ })).toBeTruthy();
        expect(screen.queryByRole('tab', { name: /^Channels$/ })).toBeNull();
        const icons = container.querySelectorAll('.dc-srv-icon');
        expect(icons[0].querySelector('img')?.getAttribute('src')).toBe('data:image/png;base64,AA');
        expect(icons[1].textContent).toBe('T');
        delete window.api.relayNodeModeration;
        delete window.api.relayNodeChannels;
        delete window.api.relayNodeChannelsSave;
    });
});
