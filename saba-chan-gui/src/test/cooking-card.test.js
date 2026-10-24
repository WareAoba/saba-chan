import { act, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { NativeProvision } from '../components/NativeProvision';
import { useInstanceMaintenance, useMaintenanceStore } from '../hooks/useInstanceMaintenance';

const server = { id: 'mc-1', name: 'Survival', module: 'minecraft', provisioning: true };
const icon = <img src="mc.png" alt="" data-testid="dish" />;

describe('서버 설치 카드 (요리)', () => {
    afterEach(() => {
        useMaintenanceStore.setState({ byId: {} });
        delete window.api.instanceProvisionProgress;
    });

    it('설치 중에는 요리하는 사바쨩과 진행 막대, 데몬이 보낸 단계와 퍼센트를 보인다', () => {
        const { container } = render(
            <NativeProvision
                server={server}
                provisionProgress={{ message: 'Downloading server files', percent: 62 }}
            />,
        );
        expect(screen.getByText('Cooking the server…')).toBeTruthy();
        expect(screen.getByText('Downloading server files')).toBeTruthy();
        expect(screen.getByText('62%')).toBeTruthy();
        expect(container.querySelector('img.saba-art-cooking')).not.toBeNull();
        expect(container.querySelector('.cook-bar-fill').style.width).toBe('62%');
    });

    it('끝나면 접시에 서버 아이콘을 올린 완성 카드를 보이고, 켜기·설정·닫기가 카드를 닫는다', () => {
        const onClose = vi.fn();
        const onStart = vi.fn();
        const onSettings = vi.fn();
        const { container } = render(
            <NativeProvision
                server={{ ...server, provisioning: false }}
                cooked
                icon={icon}
                onClose={onClose}
                onStart={onStart}
                onSettings={onSettings}
            />,
        );
        expect(screen.getByText('Done!')).toBeTruthy();
        expect(container.querySelector('img.saba-art-served')).not.toBeNull();
        expect(container.querySelector('.cook-plate [data-testid="dish"]')).not.toBeNull();

        fireEvent.click(screen.getByRole('button', { name: /Start/ }));
        expect(onClose).toHaveBeenCalledTimes(1);
        expect(onStart).toHaveBeenCalledTimes(1);

        fireEvent.click(screen.getByRole('button', { name: 'Settings first' }));
        expect(onSettings).toHaveBeenCalledTimes(1);

        fireEvent.click(screen.getByRole('button', { name: 'Close' }));
        expect(onClose).toHaveBeenCalledTimes(3);
    });

    it('실패하면 쓰러진 사바쨩과 이유를 보이고, 닫으면 설치 표시를 지운다', () => {
        const onDismiss = vi.fn();
        const { container } = render(
            <NativeProvision
                server={server}
                provisionProgress={{ done: true, error: 'Download failed (HTTP 404)' }}
                onDismiss={onDismiss}
            />,
        );
        expect(screen.getByText('Failed…')).toBeTruthy();
        expect(screen.getByText('Download failed (HTTP 404)')).toBeTruthy();
        expect(container.querySelector('img.saba-art-stopped')).not.toBeNull();
        fireEvent.click(screen.getByRole('button', { name: 'Close' }));
        expect(onDismiss).toHaveBeenCalled();
    });

    it('설치가 끝난 것을 본 서버는 데몬이 진행 표시를 지운 뒤에도 완성 카드를 남긴다', async () => {
        window.api.instanceProvisionProgress = vi.fn().mockResolvedValue({ active: true, done: true, percent: 100 });
        const { rerender } = renderHook(({ s }) => useInstanceMaintenance(s, null), { initialProps: { s: server } });
        await act(async () => {});
        expect(useMaintenanceStore.getState().byId['mc-1'].cooked).toBe(true);
        // 데몬이 5초 뒤 진행 표시를 지우면 provisioning이 false가 된다
        rerender({ s: { ...server, provisioning: false } });
        expect(useMaintenanceStore.getState().byId['mc-1'].cooked).toBe(true);
    });

    // Codex 리뷰 #546: 성공 뒤 같은 서버의 업데이트가 실패하면, 실패 표시를 닫은 뒤 예전 완성 카드가 다시 보였다
    it('새 설치·업데이트가 시작되면 예전 완성 카드를 지운다', async () => {
        useMaintenanceStore.getState().patch('mc-1', { cooked: true });
        window.api.instanceProvisionProgress = vi.fn().mockResolvedValue({ active: true, done: true, error: 'x' });
        renderHook(() => useInstanceMaintenance(server, null));
        await act(async () => {});
        expect(useMaintenanceStore.getState().byId['mc-1'].cooked).toBe(false);
    });

    // Codex 리뷰 #546: 데몬이 진행 표시를 지우기 전(provisioning=true)에는 닫아도 완성 카드가 남았다
    it('완성 카드는 데몬이 진행 표시를 지우기 전에도 닫으면 사라진다', () => {
        const { container } = render(
            <NativeProvision server={server} provisionProgress={{ done: true, percent: 100 }} cooked={false} />,
        );
        expect(container.querySelector('.cook')).toBeNull();
    });

    it('실패로 끝난 설치는 완성으로 치지 않는다', async () => {
        window.api.instanceProvisionProgress = vi.fn().mockResolvedValue({ active: true, done: true, error: 'x' });
        renderHook(() => useInstanceMaintenance(server, null));
        await act(async () => {});
        expect(useMaintenanceStore.getState().byId['mc-1'].cooked).toBeFalsy();
    });
});
