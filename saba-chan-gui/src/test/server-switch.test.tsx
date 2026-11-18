import { act, render, screen } from '@testing-library/react';
import type { ComponentType } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../components/ExtensionSlot', () => ({ default: () => null, SlotView: () => null, useSlotPages: () => [] }));
vi.mock('../components/servers/ServerConsole', () => ({ ServerConsole: () => null }));
vi.mock('../components/Modals/CommandModal', () => ({ default: () => null }));

import { ServersPage as ServersPageImpl } from '../components/servers/ServersPage';
import '../i18n';
import { useServerStore } from '../stores/useServerStore';
import { useUIStore } from '../stores/useUIStore';

const ServersPage = ServersPageImpl as unknown as ComponentType<Record<string, unknown>>;

const A = { id: 'srv-a', name: 'A', module: 'mc', status: 'running', port: 25565 };
const B = { id: 'srv-b', name: 'B', module: 'mc', status: 'running', port: 25566 };
const mc = { name: 'mc', protocols: { supported: ['stdin'], default: 'stdin' }, lists: [{ id: 'whitelist' }] };

const page = () => (
    <ServersPage
        servers={[A, B]}
        modules={[mc]}
        onAddServer={vi.fn()}
        drag={{ cardRefs: { current: {} } }}
        consoleProps={{ consoles: {}, popoutOpen: {}, consolePopoutIds: [] }}
    />
);

// 리뷰 11-18: 서버를 바꾸면 상세가 앞 서버의 화면 상태(보던 탭·주소 상자의 확인 결과)를 새 서버에 이어 붙이지 않는다
describe('목록에서 다른 서버를 고를 때', () => {
    beforeEach(() => {
        useUIStore.getState()._resetForTest();
        useServerStore.setState({ modulesLoaded: true });
    });

    it('앞 서버에서 보던 플레이어 탭을 새 서버로 한 번 그리지 않는다 (접속자 조회가 나가지 않는다)', async () => {
        useUIStore.setState({ activeServerId: A.id, requestedServerTab: 'players' } as never);
        const view = await act(async () => render(page()));
        expect(window.api.instancePlayers).toHaveBeenCalledWith(A.id);
        vi.mocked(window.api.instancePlayers).mockClear();
        vi.mocked(window.api.instanceLists).mockClear();
        await act(async () => useUIStore.setState({ activeServerId: B.id }));
        view.rerender(page());
        expect(window.api.instancePlayers).not.toHaveBeenCalled();
        expect(window.api.instanceLists).not.toHaveBeenCalled();
    });

    it('앞 서버의 늦은 포트 확인 결과를 새 서버의 주소 상자에 붙이지 않는다', async () => {
        let finish: (v: unknown) => void = () => {};
        window.api.instancePortCheck = vi.fn(() => new Promise((r) => (finish = r))) as never;
        window.api.instanceAddress = vi.fn(async (id: string) => ({
            address: '203.0.113.5',
            ports: [{ field: 'port', port: id === A.id ? A.port : B.port }],
        })) as never;
        // A는 막혀 있다는 마지막 결과가 있고, B는 확인한 적이 없다
        window.api.instancePortCheckLast = vi.fn(async (id: string) =>
            id === A.id ? { verdict: 'closed', checkedAt: '2026-11-18T00:00:00Z' } : {},
        ) as never;
        useUIStore.setState({ activeServerId: A.id } as never);
        await act(async () => render(page()));
        const recheck = document.querySelector<HTMLButtonElement>('.server-address-retry');
        expect(recheck).toBeTruthy();
        await act(async () => recheck?.click());
        await act(async () => useUIStore.setState({ activeServerId: B.id }));
        await act(async () => finish({ verdict: 'open', checkedAt: '2026-11-18T00:01:00Z' }));
        expect(screen.getByText('203.0.113.5:25566')).toBeTruthy();
        expect(document.querySelector('.server-address-check')).toBeNull();
    });
});
