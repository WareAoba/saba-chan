import type { ComponentProps, ComponentType } from 'react';
import { render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

vi.mock('../components/ExtensionSlot', () => ({ default: () => null, SlotView: () => null, useSlotPages: () => [] }));
vi.mock('../components/servers/ServerConsole', () => ({ ServerConsole: () => null }));
vi.mock('../components/Modals/CommandModal', () => ({ default: () => null }));

import { ServerDetail as ServerDetailFull } from '../components/servers/ServerDetail';
import { useMaintenanceStore } from '../hooks/useInstanceMaintenance';
import { useServerStore } from '../stores/useServerStore';

const ServerDetail = ServerDetailFull as unknown as ComponentType<Partial<ComponentProps<typeof ServerDetailFull>>>;

const detail = (status: string) => {
    const server = { id: 'srv-1', name: 'Cooked', module: 'mc', status };
    return render(
        <ServerDetail
            server={server as never}
            moduleData={{ name: 'mc' } as never}
            modules={[]}
            servers={[server] as never}
            consoleProps={{ consoles: {}, popoutOpen: {}, consolePopoutIds: [] } as never}
            settingsProps={{} as never}
        />,
    );
};

// 사용자 11-06 #10: 설치 완성 카드의 '시작'은 서버가 켜지면 남지 않는다
describe('설치 완성 카드', () => {
    it('정지된 서버에서는 보이고, 서버가 켜지면 닫힌다', () => {
        useServerStore.setState({ modulesLoaded: true });
        useMaintenanceStore.getState().patch('srv-1', { cooked: true });
        const stopped = detail('stopped');
        expect(stopped.container.querySelector('.cook-done')).not.toBeNull();
        stopped.unmount();

        detail('running');
        expect(document.querySelector('.cook-done')).toBeNull();
        expect(useMaintenanceStore.getState().byId['srv-1'].cooked).toBe(false);
    });
});
