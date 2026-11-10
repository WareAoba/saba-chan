import type { ComponentProps, ComponentType } from 'react';
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

vi.mock('../components/ExtensionSlot', () => ({ default: () => null, SlotView: () => null, useSlotPages: () => [] }));
vi.mock('../components/servers/ServerConsole', () => ({ ServerConsole: () => null }));
vi.mock('../components/Modals/CommandModal', () => ({ default: () => null }));

import { ServerDetail as ServerDetailFull } from '../components/servers/ServerDetail';
import { useServerStore } from '../stores/useServerStore';
import { useUIStore } from '../stores/useUIStore';

const ServerDetail = ServerDetailFull as unknown as ComponentType<Partial<ComponentProps<typeof ServerDetailFull>>>;

// 사용자 11-06 #27: 정보 탭의 개발 용어(통신 방식·PID·REST)는 '자세히' 안에, 인스턴스 ID는 그대로 보인다
describe('서버 정보 탭', () => {
    it('통신 방식·PID·REST는 접힌 자세히 안에 두고, 인스턴스 ID는 바로 보인다', () => {
        useServerStore.setState({ modulesLoaded: true });
        const server = {
            id: 'srv-1',
            name: 'Info',
            module: 'mc',
            status: 'running',
            pid: 4321,
            port: 25565,
            rest_port: 8212,
        };
        useUIStore.setState({ requestedServerTab: 'info' } as never);
        const { container } = render(
            <ServerDetail
                server={server as never}
                moduleData={{ name: 'mc', protocols: { supported: ['stdin'], default: 'stdin' } } as never}
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
        const more = container.querySelector('details.info-more') as HTMLDetailsElement;
        expect(more).toBeTruthy();
        expect(more.open).toBe(false);
        expect(more.querySelector('summary')!.textContent).toBe('Details');
        const hidden = [...more.querySelectorAll('dt')].map((d) => d.textContent);
        expect(hidden).toEqual(['PID', 'REST', 'Protocol']);
        const shown = [...container.querySelectorAll('.info-tab > .info-grid dt')].map((d) => d.textContent);
        expect(shown).toContain('Instance ID');
        expect(shown).toContain('Port');
        expect(shown).not.toContain('Protocol');
        expect(screen.getByText('srv-1')).toBeTruthy();
    });
});
