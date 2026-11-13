import type { ComponentProps, ComponentType } from 'react';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('../components/ExtensionSlot', () => ({ default: () => null, SlotView: () => null, useSlotPages: () => [] }));
vi.mock('../components/servers/ServerConsole', () => ({ ServerConsole: () => null }));
vi.mock('../components/Modals/CommandModal', () => ({ default: () => null }));

import { ServerDetail as ServerDetailFull } from '../components/servers/ServerDetail';
import { useMaintenanceStore } from '../hooks/useInstanceMaintenance';
import { useServerStore } from '../stores/useServerStore';
import { useUIStore } from '../stores/useUIStore';

const ServerDetail = ServerDetailFull as unknown as ComponentType<Partial<ComponentProps<typeof ServerDetailFull>>>;

// 낮 12시(UTC): 시험을 돌리는 곳의 시간대가 달라도 같은 날짜로 보인다
const NOV_3 = Date.UTC(new Date().getUTCFullYear(), 10, 3, 12) / 1000;
const NOV_11 = Date.UTC(new Date().getUTCFullYear(), 10, 11, 12) / 1000;

const server = {
    id: 'pal-1',
    name: 'Weekend',
    module: 'pal',
    status: 'stopped',
    port: 8211,
    extension_data: { install_method_steamcmd: true },
    server_build: { id: '19283746', time: NOV_3 },
};

function show(onStart = async () => {}) {
    useServerStore.setState({ modulesLoaded: true, startServer: onStart });
    useUIStore.setState({ requestedServerTab: 'info' } as never);
    return render(
        <ServerDetail
            server={server as never}
            moduleData={{ name: 'pal', install: { method: 'steamcmd' }, protocols: { supported: ['rest'], default: 'rest' } } as never}
            modules={[]}
            servers={[server] as never}
            consoleProps={{ consoles: {}, popoutOpen: {}, consolePopoutIds: [] } as never}
        />,
    );
}

const facts = (container: HTMLElement) =>
    Object.fromEntries(
        [...container.querySelectorAll('.info-tab > .ss-card:first-child .ss-rows > .info-facts .info-fact')].map((row) => [
            row.querySelector('dt')?.textContent,
            row.querySelector('dd')?.textContent,
        ]),
    );

// 시안 11: 설치 방식 익스텐션으로 받은 서버는 데몬이 적어 둔 빌드 번호와 받은 날짜를 설치된 버전으로 보인다
describe('설치된 빌드 표시', () => {
    afterEach(() => useMaintenanceStore.setState({ byId: {} }));

    it('이름 아래와 정보 탭에 빌드 번호를, 정보 탭에는 받은 날짜도 보인다', () => {
        const { container } = show();
        expect(container.querySelector('.detail-sub .tag')?.textContent).toBe('Build 19283746');
        expect(facts(container).Version).toBe('Build 19283746 (November 3)');
        expect(facts(container)['New version']).toBeUndefined();
        expect(container.querySelector('.update-notice')).toBeNull();
    });

    it('새 빌드가 있으면 표시와 안내 줄, 정보 탭의 새 버전 줄로 알리고 안내 줄에서 업데이트한다', async () => {
        const instanceApplyUpdate = vi.fn().mockResolvedValue({ success: true });
        window.api = { ...window.api, instanceApplyUpdate } as never;
        useMaintenanceStore.getState().patch('pal-1', {
            updateInfo: { update_available: true, local_buildid: '19283746', remote_buildid: '19402210', remote_build_time: NOV_11 },
        });
        const onStart = vi.fn(async () => {});
        const { container } = show(onStart);
        expect([...container.querySelectorAll('.detail-sub .tag')].map((tag) => tag.textContent)).toEqual([
            'Build 19283746',
            'New version available',
        ]);
        const notice = container.querySelector('.update-notice') as HTMLElement;
        expect(notice.textContent).toContain('Build 19283746 → 19402210 (November 11)');
        expect(facts(container)['New version']).toBe('Build 19402210 (November 11)');

        // 켜기는 그대로 머리에 있다: 업데이트하지 않고도 켤 수 있다
        fireEvent.click(screen.getAllByRole('button', { name: 'Start' })[0]);
        expect(onStart).toHaveBeenCalled();
        await act(async () => fireEvent.click(notice.querySelector('button') as HTMLElement));
        expect(instanceApplyUpdate).toHaveBeenCalledWith('pal-1');
    });
});
