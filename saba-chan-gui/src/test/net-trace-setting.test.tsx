import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { NetTraceSetting } from '../components/Modals/NetTraceSetting';
import { useSettingsStore } from '../stores/useSettingsStore';

describe('설정 > 게임 서버: 포트·서버별 네트워크 (/api/system/net-trace)', () => {
    beforeEach(() => {
        window.api = {
            systemNetTrace: vi.fn().mockResolvedValue({ success: true, access: 'needs_grant', enabled: false }),
            systemNetTraceGrant: vi.fn(),
            systemNetTraceRevoke: vi.fn(),
        } as unknown as typeof window.api;
        useSettingsStore.setState({ systemMonitorNetTrace: false });
    });

    it('기본은 꺼져 있고, 켜면 설정에 남기며 권한이 없으면 받기 버튼을 보인다', async () => {
        await act(async () => render(<NetTraceSetting />));
        expect(screen.queryByRole('button', { name: 'Grant permission' })).toBeNull();
        fireEvent.click(screen.getByRole('checkbox', { name: 'Per-server and per-port network' }));
        expect(useSettingsStore.getState().systemMonitorNetTrace).toBe(true);
        // 관리자 확인 창이 뜨는 버튼에는 Windows 방패를 붙인다
        expect(screen.getByRole('button', { name: 'Grant permission' }).querySelector('svg')).not.toBeNull();
    });

    it('권한 받기는 데몬이 관리자 확인을 띄우고, 돌려준 상태를 보인다', async () => {
        useSettingsStore.setState({ systemMonitorNetTrace: true });
        vi.mocked(window.api.systemNetTraceGrant).mockResolvedValue({
            success: true,
            access: 'needs_relogin',
            enabled: true,
            revocable: true,
        });
        await act(async () => render(<NetTraceSetting />));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Grant permission' })));
        expect(window.api.systemNetTraceGrant).toHaveBeenCalled();
        expect(screen.getByText(/sign in to Windows again/i)).toBeInTheDocument();
        expect(screen.getByRole('button', { name: 'Remove permission' })).toBeInTheDocument();
    });

    it('관리자 확인을 받지 못하면 이유를 보이고 상태는 그대로 둔다', async () => {
        useSettingsStore.setState({ systemMonitorNetTrace: true });
        vi.mocked(window.api.systemNetTraceGrant).mockResolvedValue({
            success: false,
            error_code: 'net_trace_elevation_failed',
            error: 'cancelled',
        });
        await act(async () => render(<NetTraceSetting />));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Grant permission' })));
        expect(screen.getByRole('alert')).toHaveTextContent('Administrator confirmation failed');
        expect(screen.getByRole('button', { name: 'Grant permission' })).toBeInTheDocument();
    });

    it('권한이 있으면 상태 문구 없이, 사바쨩이 준 권한일 때만 되돌리기 버튼을 보인다', async () => {
        useSettingsStore.setState({ systemMonitorNetTrace: true });
        vi.mocked(window.api.systemNetTrace).mockResolvedValue({
            success: true,
            access: 'ok',
            enabled: true,
            revocable: true,
        });
        const { unmount } = await act(async () => render(<NetTraceSetting />));
        expect(screen.getByRole('button', { name: 'Remove permission' })).toBeInTheDocument();
        expect(screen.queryByText(/Permission granted|net_trace\./i)).toBeNull();
        unmount();

        // 사바쨩 밖에서 받은 권한은 되돌리지 않으므로 아무것도 보이지 않는다
        vi.mocked(window.api.systemNetTrace).mockResolvedValue({
            success: true,
            access: 'ok',
            enabled: true,
            revocable: false,
        });
        await act(async () => render(<NetTraceSetting />));
        expect(screen.queryByRole('button', { name: 'Remove permission' })).toBeNull();
        expect(screen.queryByText(/Permission granted|net_trace\./i)).toBeNull();
    });

    it('Windows가 아니면 보이지 않는다', async () => {
        vi.mocked(window.api.systemNetTrace).mockResolvedValue({ success: true, access: 'unsupported', enabled: false });
        const { container } = await act(async () => render(<NetTraceSetting />));
        expect(container).toBeEmptyDOMElement();
    });
});
