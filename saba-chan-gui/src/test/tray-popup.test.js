/**
 * 트레이 팝업: 트레이 아이콘을 누르면 뜨는 작은 창. 데몬 상태와 서버 목록을 보이고,
 * 서버 시작·정지는 데몬 API로, 창 열기·CLI로 전환·데몬 켜고 끄기·완전히 종료는 셸의 tray_action으로 보낸다.
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { TrayPopup } from '../components/shell/TrayPopup';

const flush = () => act(async () => {});

describe('트레이 팝업 브리지', () => {
    it('trayAction과 trayPopupFit은 셸 명령으로 간다', async () => {
        const invoke = vi.fn().mockResolvedValue({ ok: true });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } });
        await api.trayAction('open_window');
        expect(invoke).toHaveBeenLastCalledWith('tray_action', { action: 'open_window' });
        await api.trayPopupFit(212);
        expect(invoke).toHaveBeenLastCalledWith('tray_popup_fit', { height: 212 });
    });
});

describe('트레이 팝업', () => {
    beforeEach(() => {
        window.api.daemonStatus = vi.fn().mockResolvedValue({ running: true });
        window.api.clientsLaunchable = vi.fn().mockResolvedValue({ cli: true, gui: true });
        window.api.serverList = vi.fn().mockResolvedValue({
            servers: [
                { id: 'a', name: '야생 서버', status: 'running', players: { online: 3, max: 20 } },
                { id: 'b', name: '팰월드', status: 'stopped' },
            ],
        });
        window.api.instanceStart = vi.fn().mockResolvedValue({ success: true });
        window.api.instanceStop = vi.fn().mockResolvedValue({ success: true });
        window.api.trayAction = vi.fn().mockResolvedValue({ ok: true });
        window.api.trayPopupFit = vi.fn();
    });
    afterEach(() => {
        for (const k of [
            'daemonStatus',
            'clientsLaunchable',
            'serverList',
            'instanceStart',
            'instanceStop',
            'trayAction',
            'trayPopupFit',
            'daemonTarget',
            'remoteDaemons',
        ])
            delete window.api[k];
    });

    it('데몬 상태와 서버를 보이고, 접속자 수는 켜진 서버에만', async () => {
        render(<TrayPopup />);
        await flush();
        expect(screen.getByText('Daemon running')).toBeTruthy();
        expect(screen.getByText('야생 서버')).toBeTruthy();
        expect(screen.getByText('3/20')).toBeTruthy();
        expect(screen.getByText('팰월드')).toBeTruthy();
        expect(window.api.trayPopupFit).toHaveBeenCalled();
    });

    it('서버 시작·정지는 데몬 API로 보낸다', async () => {
        render(<TrayPopup />);
        await flush();
        fireEvent.click(screen.getByRole('button', { name: 'Stop 야생 서버' }));
        fireEvent.click(screen.getByRole('button', { name: 'Start 팰월드' }));
        await flush();
        expect(window.api.instanceStop).toHaveBeenCalledWith('a');
        expect(window.api.instanceStart).toHaveBeenCalledWith('b');
    });

    it('창 열기·CLI로 전환·데몬 끄기·완전히 종료는 셸에 맡긴다', async () => {
        render(<TrayPopup />);
        await flush();
        for (const [name, action] of [
            ['Open window', 'open_window'],
            ['Switch to CLI', 'switch_to_cli'],
            ['Stop daemon', 'stop_daemon'],
            ['Quit completely', 'quit_all'],
        ]) {
            fireEvent.click(screen.getByRole('button', { name }));
            expect(window.api.trayAction).toHaveBeenLastCalledWith(action);
        }
    });

    it('데몬이 꺼져 있으면 서버 목록 대신 켜기를 보인다', async () => {
        window.api.daemonStatus = vi.fn().mockResolvedValue({ running: false });
        window.api.serverList = vi.fn().mockResolvedValue({ error: 'down' });
        render(<TrayPopup />);
        await flush();
        expect(screen.getByText('Daemon stopped')).toBeTruthy();
        expect(screen.queryByText('야생 서버')).toBeNull();
        fireEvent.click(screen.getByRole('button', { name: 'Start daemon' }));
        expect(window.api.trayAction).toHaveBeenLastCalledWith('start_daemon');
    });

    it('데몬 상태를 모르는 동안에는 데몬 켜기·끄기를 그리지 않는다', async () => {
        window.api.daemonStatus = vi.fn(() => new Promise(() => {}));
        render(<TrayPopup />);
        await flush();
        expect(screen.queryByRole('button', { name: 'Stop daemon' })).toBeNull();
        expect(screen.queryByRole('button', { name: 'Start daemon' })).toBeNull();
        expect(screen.getByRole('button', { name: 'Quit completely' })).toBeTruthy();
    });

    it('CLI를 띄울 수 없는 PC에서는 CLI로 전환을 그리지 않는다', async () => {
        window.api.clientsLaunchable = vi.fn().mockResolvedValue({ cli: false, gui: true });
        render(<TrayPopup />);
        await flush();
        expect(screen.queryByRole('button', { name: 'Switch to CLI' })).toBeNull();
    });

    it('Esc는 팝업을 닫는다', async () => {
        render(<TrayPopup />);
        await flush();
        fireEvent.keyDown(window, { key: 'Escape' });
        expect(window.api.trayAction).toHaveBeenLastCalledWith('close');
    });
    describe('원격 데몬을 고른 동안', () => {
        beforeEach(() => {
            window.api.daemonTarget = vi.fn().mockResolvedValue({ target: 'host-1' });
            window.api.remoteDaemons = vi.fn().mockResolvedValue({ daemons: [{ hostId: 'host-1', name: '거실 PC' }] });
        });

        it('상태 칩은 목록을 준 원격 데몬의 이름과 상태를 보인다', async () => {
            render(<TrayPopup />);
            await flush();
            expect(screen.getByText('거실 PC connected')).toBeTruthy();
            expect(screen.queryByText('Daemon running')).toBeNull();
            expect(screen.getByText('야생 서버')).toBeTruthy();
        });

        it('원격 데몬에 닿지 않으면 그렇게 보이고 지난 목록을 남기지 않는다', async () => {
            window.api.serverList = vi.fn().mockResolvedValue({ error: 'unreachable' });
            render(<TrayPopup />);
            await flush();
            expect(screen.getByText("Can't reach 거실 PC")).toBeTruthy();
            expect(screen.queryByText('야생 서버')).toBeNull();
            expect(screen.queryByText('No servers yet')).toBeNull();
        });

        it('원격 서버 정지는 한 번 더 눌러야 보낸다', async () => {
            render(<TrayPopup />);
            await flush();
            fireEvent.click(screen.getByRole('button', { name: 'Stop 야생 서버' }));
            await flush();
            expect(window.api.instanceStop).not.toHaveBeenCalled();
            fireEvent.click(screen.getByRole('button', { name: 'Confirm stop 야생 서버' }));
            await flush();
            expect(window.api.instanceStop).toHaveBeenCalledWith('a');
        });

        it('원격 서버 시작은 확인 없이 보낸다', async () => {
            render(<TrayPopup />);
            await flush();
            fireEvent.click(screen.getByRole('button', { name: 'Start 팰월드' }));
            await flush();
            expect(window.api.instanceStart).toHaveBeenCalledWith('b');
        });
    });
});
