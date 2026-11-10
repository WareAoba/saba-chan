import type { ComponentProps, ComponentType } from 'react';
import { act, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const slotRenders = vi.hoisted(() => ({ count: 0 }));

vi.mock('../components/ExtensionSlot', () => ({
    default: () => {
        slotRenders.count++;
        return null;
    },
    SlotView: () => null,
    useSlotPages: () => [],
}));
vi.mock('../components/servers/ServerConsole', () => ({ ServerConsole: () => null }));
vi.mock('../components/Modals/CommandModal', () => ({ default: () => null }));

import { ServerDetail as ServerDetailFull } from '../components/servers/ServerDetail';
import { useServerStore } from '../stores/useServerStore';

// 이 시험이 보는 props만 넘기므로 나머지는 선택으로 느슨하게 받는다
const ServerDetail = ServerDetailFull as unknown as ComponentType<Partial<ComponentProps<typeof ServerDetailFull>>>;

describe('가동 시간 표시', () => {
    it('1초마다 가동 시간 글자만 바뀌고 상세 전체는 다시 그리지 않는다', () => {
        const now = Math.floor(Date.now() / 1000);
        useServerStore.setState({ nowEpoch: now, modulesLoaded: true });
        const server = { id: 'srv-1', name: 'Uptime', module: 'mc', status: 'running', start_time: now - 7384 };
        render(
            <ServerDetail
                server={server as never}
                moduleData={{ name: 'mc' } as never}
                modules={[]}
                servers={[server] as never}
                setServers={() => {}}
                onStart={() => {}}
                onStop={() => {}}
                setModal={() => {}}
                consoleProps={{ consoles: {}, popoutOpen: {}, consolePopoutIds: [] } as never}
                settingsProps={{} as never}
            />,
        );
        expect(screen.getAllByText('02:03:04').length).toBeGreaterThan(0);
        const before = slotRenders.count;

        act(() => useServerStore.setState({ nowEpoch: now + 1 }));

        expect(screen.getAllByText('02:03:05').length).toBeGreaterThan(0);
        expect(slotRenders.count).toBe(before);
    });
});
