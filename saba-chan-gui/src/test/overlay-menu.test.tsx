/**
 * 펫 우클릭 판: 펫을 우클릭하면 셸이 펫 옆에 띄우는 작은 창 (셸 overlay.rs `toggle_menu`).
 * 서버 켜고 끄기(정지는 한 번 더 눌러 확정), 펫 크기 막대, 앱 열기, 바탕화면에서 숨기기.
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { OverlayMenu } from '../components/shell/OverlayMenu';

const flush = () => act(async () => {});

describe('펫 우클릭 판', () => {
    beforeEach(() => {
        window.api.daemonStatus = vi.fn().mockResolvedValue({ running: true });
        window.api.daemonTarget = vi.fn().mockResolvedValue({ target: null });
        window.api.serverList = vi.fn().mockResolvedValue({
            servers: [
                { id: 'a', name: '야생 서버', status: 'running', players: { online: 3, max: 20 } },
                { id: 'b', name: '팰월드', status: 'stopped' },
            ],
        });
        window.api.instanceStart = vi.fn().mockResolvedValue({ success: true });
        window.api.instanceStop = vi.fn().mockResolvedValue({ success: true });
        window.api.overlayAction = vi.fn().mockResolvedValue({ ok: true });
        window.api.overlayMenuFit = vi.fn();
        window.api.overlaySetScale = vi.fn().mockResolvedValue(150);
    });
    afterEach(() => {
        for (const k of [
            'daemonStatus',
            'daemonTarget',
            'serverList',
            'instanceStart',
            'instanceStop',
            'overlayAction',
            'overlayMenuFit',
            'overlaySetScale',
        ])
            delete (window.api as unknown as Record<string, unknown>)[k];
        window.history.replaceState(null, '', '/');
    });

    it('서버를 보이고 높이를 셸에 알린다', async () => {
        render(<OverlayMenu />);
        await flush();
        expect(screen.getByText('야생 서버')).toBeTruthy();
        expect(screen.getByText('팰월드')).toBeTruthy();
        expect(window.api.overlayMenuFit).toHaveBeenCalled();
    });

    it('꺼진 서버는 한 번에 켜고, 켜진 서버는 한 번 더 눌러야 끈다', async () => {
        render(<OverlayMenu />);
        await flush();
        await act(async () => {
            fireEvent.click(screen.getByRole('button', { name: /Start 팰월드/ }));
        });
        expect(window.api.instanceStart).toHaveBeenCalledWith('b');
        fireEvent.click(screen.getByRole('button', { name: /Stop 야생 서버/ }));
        expect(window.api.instanceStop).not.toHaveBeenCalled();
        await act(async () => {
            fireEvent.click(screen.getByRole('button', { name: /야생 서버/ }));
        });
        expect(window.api.instanceStop).toHaveBeenCalledWith('a');
    });

    it('크기 막대는 주소의 지금 크기에서 시작하고, 움직이면 셸에 알린다', async () => {
        window.history.replaceState(null, '', '/?overlay-menu=1&scale=120');
        render(<OverlayMenu />);
        await flush();
        const slider = screen.getByRole('slider') as HTMLInputElement;
        expect(slider.value).toBe('120');
        fireEvent.change(slider, { target: { value: '150' } });
        expect(window.api.overlaySetScale).toHaveBeenCalledWith(150);
        expect(screen.getByText('150%')).toBeTruthy();
    });

    it('앱 열기·숨기기는 셸 동작으로, Esc는 판을 닫는다', async () => {
        render(<OverlayMenu />);
        await flush();
        fireEvent.click(screen.getByRole('button', { name: /Open Saba-chan/ }));
        expect(window.api.overlayAction).toHaveBeenLastCalledWith('open_app');
        fireEvent.click(screen.getByRole('button', { name: /Hide from desktop/ }));
        expect(window.api.overlayAction).toHaveBeenLastCalledWith('hide');
        fireEvent.keyDown(window, { key: 'Escape' });
        expect(window.api.overlayAction).toHaveBeenLastCalledWith('close');
    });
});
