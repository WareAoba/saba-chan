/**
 * 브랜치 서버: 서버 줄을 우클릭해 그 서버 아래에 묶이는 새 서버를 만든다. 설정은 만들 때 데몬이 한 번 옮기고,
 * 목록에서는 상위 서버 바로 아래에 가지로 붙어 접을 수 있다. 브랜치는 한 단계만 둔다.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import i18n from 'i18next';
import type { ComponentType } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../components/ExtensionSlot', () => ({ default: () => null, SlotView: () => null, useSlotPages: () => [] }));
vi.mock('../components/servers/ServerConsole', () => ({ ServerConsole: () => null }));
vi.mock('../components/Modals/CommandModal', () => ({ default: () => null }));

import { ServersPage as ServersPageImpl } from '../components/servers/ServersPage';
import { ContextMenuProvider } from '../components/ui/ContextMenu';
import { setupServerStore } from './serverStore';
import { useServerStore } from '../stores/useServerStore';
import { useSettingsStore } from '../stores/useSettingsStore';
import { useUIStore } from '../stores/useUIStore';

/** 시험에 필요한 props만 넘기려고 느슨하게 받는다 */
const ServersPage = ServersPageImpl as unknown as ComponentType<Record<string, unknown>>;
const tr = (key: string, vars?: Record<string, unknown>) => i18n.t(`gui:branch.${key}`, vars ?? {});

const LIST = [
    { id: 's1', name: 'Friends', module: 'game', status: 'running', port: 25565 },
    { id: 's3', name: 'Weekend', module: 'other', status: 'stopped' },
    { id: 's2', name: 'Season 2', module: 'game', status: 'stopped', branch_of: 's1' },
];

function page(onCreateBranch = vi.fn().mockResolvedValue(true)) {
    useServerStore.getState().setServers(LIST as never);
    useServerStore.setState({ addServer: onCreateBranch });
    render(
        <ContextMenuProvider allowNative={false}>
            <ServersPage
                servers={useServerStore.getState().servers}
                modules={[]}
                onAddServer={() => {}}
                drag={{ cardRefs: { current: {} }, skipNextClick: { current: false }, handleCardPointerDown: () => {} }}
                consoleProps={{ consoles: {}, popoutOpen: {}, consolePopoutIds: [] }}
                settingsProps={{}}
            />
        </ContextMenuProvider>,
    );
    return onCreateBranch;
}
const rowNames = () => [...document.querySelectorAll('.server-row-name')].map((el) => el.textContent);
const row = (name: string) => [...document.querySelectorAll('.server-row')].find((el) => el.textContent?.includes(name)) as HTMLElement;

describe('브랜치 서버', () => {
    beforeEach(() => {
        localStorage.clear();
        useUIStore.getState()._resetForTest();
        useServerStore.getState()._resetForTest();
        useSettingsStore.setState({ settingsReady: true, closedGuides: ['firstRun'] });
        useServerStore.setState({ modulesLoaded: true });
        window.showToast = vi.fn();
        window.api = { instanceCheck: vi.fn().mockResolvedValue({ name: { value: 'x', taken: false }, ports: [{ field: 'port', port: 25566, in_use_by: [] }] }) } as unknown as typeof window.api;
    });

    it('목록에서 브랜치는 상위 서버 바로 아래에 붙고, 접으면 상위 서버 줄에 개수만 남는다', async () => {
        page();
        expect(rowNames()).toEqual(['Friends', 'Season 2', 'Weekend']);
        expect(row('Season 2').className).toContain('branch');

        await act(async () => fireEvent.click(screen.getByRole('button', { name: tr('collapse', { name: 'Friends' }) })));
        expect(rowNames()).toEqual(['Friends1', 'Weekend']);
        expect(screen.getByRole('img', { name: tr('count', { count: 1 }) })).toBeTruthy();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: tr('expand', { name: 'Friends' }) })));
        expect(rowNames()).toEqual(['Friends', 'Season 2', 'Weekend']);

        // 브랜치를 고르면 상세 머리에 어느 서버의 브랜치인지 보인다
        await act(async () => fireEvent.click(row('Season 2')));
        expect(document.querySelector('.detail-sub')?.textContent).toContain(tr('tag', { name: 'Friends' }));
    });

    it('우클릭 메뉴로 만든다: 데몬이 정한 빈 포트를 보이고, 이름·상위 서버·포트만 보낸다. 브랜치에는 그 메뉴가 없다', async () => {
        const create = page();
        fireEvent.contextMenu(row('Season 2'), { clientX: 10, clientY: 10 });
        expect(screen.queryByRole('menuitem', { name: tr('create') })).toBeNull();
        fireEvent.keyDown(document.body, { key: 'Escape' });

        fireEvent.contextMenu(row('Friends'), { clientX: 10, clientY: 10 });
        await act(async () => fireEvent.click(screen.getByRole('menuitem', { name: tr('create') })));
        const dialog = await screen.findByRole('dialog', { name: tr('create') });
        expect(dialog.textContent).toContain(tr('desc', { name: 'Friends' }));
        const name = screen.getByLabelText(tr('name')) as HTMLInputElement;
        expect(window.api.instanceCheck).toHaveBeenCalledWith({ name: name.value, branch_of: 's1' });
        await waitFor(() => expect(dialog.textContent).toContain(`25566${tr('port_hint')}`));

        fireEvent.change(name, { target: { value: ' Friends test ' } });
        await act(async () => fireEvent.click(screen.getByRole('button', { name: tr('submit') })));
        expect(create).toHaveBeenCalledWith({ name: 'Friends test', module_name: 'game', branch_of: 's1', port: 25566 });
        await waitFor(() => expect(screen.queryByRole('dialog', { name: tr('create') })).toBeNull());
    });

    it('만들기 요청에 상위 서버(branch_of)를 실어 데몬에 보낸다', async () => {
        window.api = {
            serverList: vi.fn().mockResolvedValue({ servers: [] }),
            instanceCreate: vi.fn().mockResolvedValue({ success: true, id: 'new' }),
        } as unknown as typeof window.api;
        const { result } = setupServerStore();
        await act(async () => {
            await result.current.addServer({ name: 'Friends test', module_name: 'game', branch_of: 's1', port: 25566 } as never);
        });
        expect(window.api.instanceCreate).toHaveBeenCalledWith({ name: 'Friends test', module_name: 'game', branch_of: 's1', port: 25566 });
    });
});
