/**
 * GUI ⇄ CLI 전환: 데몬이 이 PC에서 CLI를 띄울 수 있을 때만 버튼을 보이고,
 * 누르면 고른 서버로 CLI 메뉴를 띄운 뒤 GUI(인터페이스)만 끝낸다. 데몬은 그대로 둔다.
 * CLI가 `gui` 명령으로 GUI를 띄우면 `--open-server=<id>` → `app:openServer`로 그 서버를 연다.
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { createEventHub } from '../api/events';
import { SwitchToCliButton } from '../components/shell/SwitchToCli';
import { useUIStore } from '../stores/useUIStore';

const flush = () => act(async () => {});

describe('전환 브리지', () => {
    const setup = (data) => {
        const invoke = vi.fn().mockResolvedValue({ status: 200, data });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } });
        return { api, sent: () => invoke.mock.calls.at(-1)[1] };
    };

    it('clientsLaunchable은 GET, clientLaunch는 종류와 서버를 POST한다', async () => {
        const { api, sent } = setup({ cli: true, gui: false });
        await expect(api.clientsLaunchable()).resolves.toEqual({ cli: true, gui: false });
        expect(sent()).toMatchObject({ method: 'GET', path: '/api/clients/launch' });
        await api.clientLaunch({ kind: 'cli', instance: 'inst-1' });
        expect(sent()).toMatchObject({
            method: 'POST',
            path: '/api/clients/launch',
            body: { kind: 'cli', instance: 'inst-1' },
        });
    });

    it('화면이 구독하기 전에 온 서버 열기 요청은 첫 구독자에게 넘긴다', async () => {
        let deliver;
        const hub = createEventHub();
        await hub.start(async (name, cb) => {
            if (name === 'app:openServer') deliver = cb;
        });
        deliver({ instanceId: 'inst-1' });
        const first = vi.fn();
        hub.on('app:openServer', first);
        await Promise.resolve();
        expect(first).toHaveBeenCalledWith({ instanceId: 'inst-1' });
        const second = vi.fn();
        hub.on('app:openServer', second);
        await Promise.resolve();
        expect(second).not.toHaveBeenCalled();
    });
});

describe('CLI로 전환 버튼', () => {
    afterEach(() => {
        delete window.api.clientsLaunchable;
        delete window.api.clientLaunch;
        delete window.api.closeResponse;
        useUIStore.setState({ activeServerId: null });
    });

    it('데몬이 CLI를 띄울 수 없다고 하면 그리지 않는다', async () => {
        window.api.clientsLaunchable = vi.fn().mockResolvedValue({ cli: false, gui: true });
        const { container } = render(<SwitchToCliButton />);
        await flush();
        expect(container.innerHTML).toBe('');
    });

    it('고른 서버로 CLI를 띄우고, 띄웠으면 인터페이스만 끝낸다', async () => {
        useUIStore.setState({ activeServerId: 'inst-1' });
        window.api.clientsLaunchable = vi.fn().mockResolvedValue({ cli: true, gui: true });
        window.api.clientLaunch = vi.fn().mockResolvedValue({ success: true, pid: 10 });
        window.api.closeResponse = vi.fn();
        render(<SwitchToCliButton />);
        await flush();
        fireEvent.click(screen.getByRole('button', { name: 'Switch to CLI' }));
        await flush();
        expect(window.api.clientLaunch).toHaveBeenCalledWith({ kind: 'cli', instance: 'inst-1' });
        expect(window.api.closeResponse).toHaveBeenCalledWith('exit-interface');
    });

    it('못 띄우면 알리고 GUI는 남는다', async () => {
        window.showToast = vi.fn();
        window.api.clientsLaunchable = vi.fn().mockResolvedValue({ cli: true });
        window.api.clientLaunch = vi
            .fn()
            .mockResolvedValue({ success: false, error_code: 'launch_failed', error: 'boom' });
        window.api.closeResponse = vi.fn();
        render(<SwitchToCliButton />);
        await flush();
        fireEvent.click(screen.getByRole('button', { name: 'Switch to CLI' }));
        await flush();
        expect(window.showToast).toHaveBeenCalledWith("Couldn't open the CLI. boom", 'warning', 6000, undefined);
        expect(window.api.closeResponse).not.toHaveBeenCalled();
    });
});
