/**
 * 플레이어 탭: 모듈이 선언한 목록(화이트리스트·관리자·밴 등)의 지금 명단을 데몬에서 받아 보이고,
 * 이름을 넣고 빼는 일은 모듈이 그 목록에 붙인 명령을 그대로 보낸다. 목록을 선언하지 않은 모듈에는 탭이 없다.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import i18n from 'i18next';
import type { ComponentProps, ComponentType } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('../components/ExtensionSlot', () => ({ default: () => null, SlotView: () => null, useSlotPages: () => [] }));
vi.mock('../components/servers/ServerConsole', () => ({ ServerConsole: () => null }));
vi.mock('../components/Modals/CommandModal', () => ({ default: () => null }));

import { PlayersTab } from '../components/servers/PlayersTab';
import { ServerDetail as ServerDetailFull } from '../components/servers/ServerDetail';
import { type Server, useServerStore } from '../stores/useServerStore';
import type { ModuleInfo } from '../types/daemon';

const ServerDetail = ServerDetailFull as unknown as ComponentType<Partial<ComponentProps<typeof ServerDetailFull>>>;
const tr = (key: string, vars?: Record<string, unknown>) => i18n.t(`gui:players.${key}`, vars ?? {});

const player = { name: 'player', type: 'text', label: 'Player Name', required: true };
const moduleData = {
    name: 'game',
    settings: { fields: [{ name: 'allow_only', field_type: 'boolean', label: 'Allow list', default: false }] },
    commands: {
        fields: [
            { name: 'allow', label: 'Allow', inputs: [player] },
            { name: 'disallow', label: 'Disallow', inputs: [player] },
            { name: 'block', label: 'Block', inputs: [player, { name: 'reason', type: 'text', label: 'Reason' }] },
            { name: 'unblock', label: 'Unblock', inputs: [player] },
        ],
    },
    lists: [
        { id: 'whitelist', add: 'allow', remove: 'disallow', setting: 'allow_only' },
        { id: 'banned', add: 'block', remove: 'unblock', danger: true },
    ],
    functions: null,
} as unknown as ModuleInfo;
const running = { id: 's1', name: '친구들 서버', module: 'game', status: 'running', module_settings: {} } as unknown as Server;

const LISTS = (editable = true) => ({
    success: true,
    lists: [
        {
            id: 'whitelist',
            label: '화이트리스트',
            add: 'allow',
            remove: 'disallow',
            setting: 'allow_only',
            danger: false,
            editable,
            entries: [{ name: 'mango' }, { name: 'dubu', added: '2026-11-02' }],
        },
        {
            id: 'banned',
            label: '밴',
            add: 'block',
            remove: 'unblock',
            danger: true,
            editable,
            entries: [{ name: 'griefer99', added: '2026-11-09T21:03:11+09:00', note: '도배' }, { name: 'tnt', added: '2026-10-30T10:00:00+09:00' }],
        },
    ],
});

const day = (iso: string) => new Intl.DateTimeFormat(i18n.language, { month: 'long', day: 'numeric' }).format(new Date(iso));

afterEach(() => {
    window.api = undefined as never;
});

describe('플레이어 탭', () => {
    it('목록마다 지금 명단을 보이고, 이름을 넣고 빼면 모듈이 붙인 명령을 보낸 뒤 다시 읽는다', async () => {
        const instanceLists = vi.fn().mockResolvedValue(LISTS());
        const executeCommand = vi.fn().mockResolvedValue({ success: true });
        window.api = {
            instanceLists,
            executeCommand,
            instancePlayers: vi.fn().mockResolvedValue({ success: true, online: 1, players: [{ name: 'Mango' }] }),
        } as unknown as typeof window.api;
        render(<PlayersTab server={running} moduleData={moduleData} />);

        expect(await screen.findByRole('tab', { name: '화이트리스트 2' })).toBeTruthy();
        expect(screen.getByRole('tab', { name: '밴 2' })).toBeTruthy();
        expect(instanceLists).toHaveBeenCalledWith('s1');
        const rows = () => screen.getAllByRole('listitem').map((li) => li.textContent);
        expect(rows()).toEqual(['Mmango', `Ddubu${tr('added_on', { date: day('2026-11-02') })}`]);
        // 지금 접속해 있는 사람만 표시한다
        expect(screen.getAllByLabelText(tr('online'))).toHaveLength(1);

        fireEvent.change(screen.getByLabelText(tr('name_placeholder')), { target: { value: ' kimchi ' } });
        await act(async () => fireEvent.click(screen.getByRole('button', { name: tr('add') })));
        expect(executeCommand).toHaveBeenLastCalledWith('s1', { command: 'allow', args: { player: 'kimchi' } });
        await waitFor(() => expect(instanceLists.mock.calls.length).toBeGreaterThanOrEqual(2));
        expect((screen.getByLabelText(tr('name_placeholder')) as HTMLInputElement).value).toBe('');

        await act(async () => fireEvent.click(screen.getByRole('button', { name: tr('remove_named', { name: 'dubu' }) })));
        expect(executeCommand).toHaveBeenLastCalledWith('s1', { command: 'disallow', args: { player: 'dubu' } });

        // 막는 목록: 넣기 명령의 나머지 입력(이유)이 옆 칸으로 나오고, 넣기 단추는 목록 이름이다
        fireEvent.click(screen.getByRole('tab', { name: '밴 2' }));
        expect(rows()).toEqual([`Ggriefer99도배 · ${day('2026-11-09T21:03:11+09:00')}`, `Ttnt${tr('no_reason')} · ${day('2026-10-30T10:00:00+09:00')}`]);
        fireEvent.change(screen.getByLabelText(tr('name_placeholder')), { target: { value: 'xX_tnt_Xx' } });
        fireEvent.change(screen.getByLabelText(tr('optional', { label: 'Reason' })), { target: { value: '도배' } });
        await act(async () => fireEvent.click(screen.getByRole('button', { name: '밴' })));
        expect(executeCommand).toHaveBeenLastCalledWith('s1', { command: 'block', args: { player: 'xX_tnt_Xx', reason: '도배' } });
    });

    it('명령을 보낼 수 없는 서버(꺼진 서버)는 명단만 보이고 넣기·빼기를 막는다', async () => {
        const executeCommand = vi.fn();
        window.api = { instanceLists: vi.fn().mockResolvedValue(LISTS(false)), executeCommand } as unknown as typeof window.api;
        render(<PlayersTab server={{ ...running, status: 'stopped' } as Server} moduleData={moduleData} />);
        await screen.findByRole('tab', { name: '화이트리스트 2' });
        expect(document.body.textContent).toContain(tr('desc_offline'));
        expect((screen.getByLabelText(tr('name_placeholder')) as HTMLInputElement).disabled).toBe(true);
        expect((screen.getByRole('button', { name: tr('remove_named', { name: 'mango' }) }) as HTMLButtonElement).disabled).toBe(true);
    });

    it('목록을 쓸지 정하는 설정이 있으면 스위치로 보이고, 바꾸면 그 설정 하나를 저장한다', async () => {
        const instanceUpdateSettings = vi.fn().mockResolvedValue({ success: true });
        window.api = { instanceLists: vi.fn().mockResolvedValue(LISTS()), instanceUpdateSettings } as unknown as typeof window.api;
        render(<PlayersTab server={running} moduleData={moduleData} />);
        const toggle = (await screen.findByRole('checkbox', { name: tr('use_list', { list: '화이트리스트' }) })) as HTMLInputElement;
        expect(toggle.checked).toBe(false);
        await act(async () => fireEvent.click(toggle));
        expect(instanceUpdateSettings).toHaveBeenCalledWith('s1', { allow_only: true });
        // 설정이 없는 목록에는 스위치가 없다
        fireEvent.click(screen.getByRole('tab', { name: '밴 2' }));
        expect(screen.queryByRole('checkbox')).toBeNull();
    });

    it('모듈이 목록을 선언했을 때만 서버 상세에 플레이어 탭이 생긴다', async () => {
        useServerStore.setState({ modulesLoaded: true });
        window.api = { instanceLists: vi.fn().mockResolvedValue(LISTS()) } as unknown as typeof window.api;
        const detail = (data: ModuleInfo) =>
            render(
                <ServerDetail
                    server={running as never}
                    moduleData={data}
                    modules={[]}
                    servers={[running] as never}
                    consoleProps={{ consoles: {}, popoutOpen: {}, consolePopoutIds: [] } as never}
                    settingsProps={{} as never}
                />,
            );
        const tab = i18n.t('gui:shell.tab_players');
        const plain = detail({ ...moduleData, lists: undefined } as ModuleInfo);
        expect(screen.queryByRole('tab', { name: tab })).toBeNull();
        plain.unmount();
        detail(moduleData);
        await act(async () => fireEvent.click(screen.getByRole('tab', { name: tab })));
        expect(await screen.findByRole('tab', { name: '화이트리스트 2' })).toBeTruthy();
    });
});
