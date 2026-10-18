import { act, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const slotRenders = vi.hoisted(() => ({ count: 0 }));

vi.mock('../components/ExtensionSlot', () => ({
    default: () => {
        slotRenders.count++;
        return null;
    },
}));
vi.mock('../components/servers/ServerConsole', () => ({ ServerConsole: () => null }));
vi.mock('../components/Modals/CommandModal', () => ({ default: () => null }));

import { ServerDetail } from '../components/servers/ServerDetail';
import { useServerStore } from '../stores/useServerStore';

describe('가동 시간 표시', () => {
    it('1초마다 가동 시간 글자만 바뀌고 상세 전체는 다시 그리지 않는다', () => {
        const now = Math.floor(Date.now() / 1000);
        useServerStore.setState({ nowEpoch: now, modulesLoaded: true });
        const server = { id: 'srv-1', name: 'Uptime', module: 'mc', status: 'running', start_time: now - 7384 };
        render(
            <ServerDetail
                server={server}
                moduleData={{ name: 'mc' }}
                modules={[]}
                servers={[server]}
                setServers={() => {}}
                onStart={() => {}}
                onStop={() => {}}
                setModal={() => {}}
                consoleProps={{ consoles: {}, popoutOpen: {} }}
                settingsProps={{}}
            />,
        );
        expect(screen.getAllByText('02:03:04').length).toBeGreaterThan(0);
        const before = slotRenders.count;

        act(() => useServerStore.setState({ nowEpoch: now + 1 }));

        expect(screen.getAllByText('02:03:05').length).toBeGreaterThan(0);
        expect(slotRenders.count).toBe(before);
    });
});
