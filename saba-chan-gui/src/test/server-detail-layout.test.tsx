import type { ComponentProps, ComponentType } from 'react';
import { render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

vi.mock('../components/ExtensionSlot', () => ({ default: () => null, SlotView: () => null, useSlotPages: () => [] }));
vi.mock('../components/servers/ServerConsole', () => ({ ServerConsole: () => null }));
vi.mock('../components/Modals/CommandModal', () => ({ default: () => null }));

import { ServerDetail as ServerDetailFull } from '../components/servers/ServerDetail';
import { useServerStore } from '../stores/useServerStore';
import { useUIStore } from '../stores/useUIStore';

const ServerDetail = ServerDetailFull as unknown as ComponentType<Partial<ComponentProps<typeof ServerDetailFull>>>;

const server = { id: 'srv-1', name: 'Lay', module: 'mc', status: 'running', port: 25565 };
const open = (tab: string) => {
    useServerStore.setState({ modulesLoaded: true });
    useUIStore.setState({ requestedServerTab: tab } as never);
    return render(
        <ServerDetail
            server={server as never}
            moduleData={{ name: 'mc', protocols: { supported: ['stdin'], default: 'stdin' } } as never}
            modules={[]}
            servers={[server] as never}
            consoleProps={{ consoles: {}, popoutOpen: {}, consolePopoutIds: [] } as never}
        />,
    );
};

// 사용자 11-14: 서버 카드부터 탭 바까지 고정하고 아래만 스크롤, 콘솔 탭은 콘솔이 칸 높이에 맞는다
describe('인스턴스 화면 배치', () => {
    it('서버 카드와 탭 바는 위에 붙는 머리 안에 있고, 탭 내용은 그 밖에서 스크롤한다', () => {
        const { container } = open('info');
        const head = container.querySelector('.server-detail > .sticky-head');
        expect(head?.querySelector('.detail-head')).toBeTruthy();
        expect(head?.querySelector('.detail-tabrow')).toBeTruthy();
        expect(head?.querySelector('.detail-body')).toBeNull();
        expect(container.querySelector('.server-detail--fit')).toBeNull();
    });

    it('콘솔 탭은 상세를 칸 높이에 맞춘다', () => {
        const { container } = open('console');
        expect(container.querySelector('.server-detail.server-detail--fit > .detail-body')).toBeTruthy();
    });
});
