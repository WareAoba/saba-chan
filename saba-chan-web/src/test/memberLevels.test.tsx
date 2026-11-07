import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { GuildPermissions } from '../components/PermissionsTab';
import { initI18n } from '../i18n';
import { mockApi } from './mocks';

// 사용자 지정(2026-11-07, 앱 멤버 탭과 같다): 레벨 칸마다 사람 칩, 윗줄 닉네임 찾기(10명씩, 끌어서 레벨에 넣기),
// 사람 추가는 닉네임 찾기로, 소유자 칩은 강조
const flush = () => act(async () => {});
const many = Array.from({ length: 14 }, (_, i) => ({
    id: String(2000 + i),
    username: `bot${i}`,
    displayName: `Bot ${String(i).padStart(2, '0')}`,
}));

function setup() {
    const api = new Proxy(
        {
            permissionsGet: vi.fn().mockResolvedValue({
                guildId: 'g1',
                defaultLevel: 1,
                ownerDiscordId: '1',
                members: { 3: 2 },
            }),
            permissionsCommands: vi.fn().mockResolvedValue({ instances: [], items: [] }),
            relayListNodeMembers: vi.fn().mockResolvedValue({
                members: [
                    { id: '1', username: 'acorn', displayName: 'Acorn' },
                    { id: '2', username: 'birch', displayName: 'Birch' },
                    { id: '3', username: 'cedar', displayName: 'Cedar' },
                    ...many,
                ],
            }),
            permissionsSetMember: vi.fn().mockResolvedValue({ ok: true }),
            guildStartNoticeGet: vi.fn().mockResolvedValue({ guildId: 'g1', instances: [], allowedInstances: null }),
        },
        { get: (t, k) => t[k as keyof typeof t] ?? vi.fn().mockResolvedValue({}) },
    );
    render(<GuildPermissions api={mockApi(api)} guildId="g1" />);
    return api;
}

async function openMembers() {
    const api = setup();
    await flush();
    await flush();
    fireEvent.click(screen.getByRole('tab', { name: 'Member levels' }));
    // 가짜 api 는 모든 이름에 답하므로(then 도) 그대로 돌려주면 await 가 끝나지 않는다
    return { api };
}
const row = (name: string) =>
    within(document.querySelector('.level-rows') as HTMLElement).getByRole('listitem', { name });

describe('웹 멤버 탭', () => {
    beforeAll(() => initI18n('en'));
    afterEach(cleanup);

    it('레벨 칸마다 사람 칩, 소유자 칩은 강조, 기본 레벨 사람은 접어 둔다', async () => {
        await openMembers();
        expect(within(row('Owner')).getByText('Acorn').closest('.person-chip')?.classList.contains('is-owner')).toBe(
            true,
        );
        expect(within(row('Admin')).getByRole('button', { name: 'Cedar' })).toBeTruthy();
        expect(within(row('User')).queryByRole('button', { name: 'Birch' })).toBeNull();
        fireEvent.click(within(row('User')).getByRole('button', { name: /Show 15 on the default level/ }));
        expect(within(row('User')).getByRole('button', { name: 'Birch' })).toBeTruthy();
    });

    it('닉네임으로 찾으면 10명씩, 끝까지 내리면 더 보인다', async () => {
        await openMembers();
        fireEvent.change(screen.getByRole('combobox', { name: 'Find a member' }), { target: { value: 'bot' } });
        const list = screen.getByRole('listbox', { name: 'Find a member' });
        expect(within(list).getAllByRole('option')).toHaveLength(10);
        Object.defineProperties(list, {
            scrollTop: { value: 200, configurable: true },
            clientHeight: { value: 100, configurable: true },
            scrollHeight: { value: 300, configurable: true },
        });
        fireEvent.scroll(list);
        expect(within(list).getAllByRole('option')).toHaveLength(14);
        fireEvent.click(screen.getByRole('button', { name: 'Clear search' }));
        expect(screen.queryByRole('listbox')).toBeNull();
    });

    it('찾은 사람을 레벨 칸에 끌어 놓으면 그 레벨이 된다', async () => {
        const { api } = await openMembers();
        fireEvent.change(screen.getByRole('combobox', { name: 'Find a member' }), { target: { value: 'birch' } });
        const birch = within(screen.getByRole('listbox')).getByRole('option', { name: 'Birch' });
        document.elementFromPoint = () => row('Admin');
        try {
            fireEvent.pointerDown(birch, { clientX: 0, clientY: 0, button: 0 });
            fireEvent.pointerMove(window, { clientX: 0, clientY: 40 });
            expect(row('Admin').classList.contains('is-over')).toBe(true);
            fireEvent.pointerUp(window, { clientX: 0, clientY: 40 });
        } finally {
            delete (document as Partial<Document>).elementFromPoint;
        }
        await flush();
        expect(api.permissionsSetMember).toHaveBeenCalledWith('g1', '2', 2);
    });

    it('사람 추가는 닉네임 찾기로 고른다', async () => {
        const { api } = await openMembers();
        fireEvent.click(screen.getByRole('button', { name: 'Add person to Guest' }));
        expect(screen.getByRole('button', { name: 'Add person to Guest' }).classList.contains('is-active')).toBe(true);
        fireEvent.change(screen.getByRole('combobox', { name: 'Find a member to add to Guest' }), {
            target: { value: 'ced' },
        });
        fireEvent.click(screen.getByRole('option', { name: 'Cedar' }));
        await flush();
        expect(api.permissionsSetMember).toHaveBeenCalledWith('g1', '3', 0);
    });
});
