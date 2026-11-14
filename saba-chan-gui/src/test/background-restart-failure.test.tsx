/**
 * 배경 팝오버에서 데몬 재시작이 실패하면 콘솔에만 남기지 않고 알린다 (G-U7, DaemonDown과 같은 문구).
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import i18n from 'i18next';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import BackgroundModal from '../components/Modals/BackgroundModal';
import { useUIStore } from '../stores/useUIStore';

beforeEach(() => {
    window.showToast = vi.fn();
    useUIStore.setState({ backgroundDaemonStatus: 'stopped' });
});

const failed = (error: string) => i18n.t('gui:shell.daemon_restart_failed', { error });

describe('배경 팝오버의 데몬 재시작 실패', () => {
    it('데몬이 실패로 답하면 이유를 알린다', async () => {
        window.api.daemonRestart = vi.fn().mockResolvedValue({ success: false, error: 'port in use' }) as never;
        render(<BackgroundModal onClose={vi.fn()} />);
        const button = await screen.findByRole('button', { name: i18n.t('gui:background_modal.restart_button') });
        await act(async () => fireEvent.click(button));
        expect(window.showToast).toHaveBeenCalledWith(failed('port in use'), 'error', 6000, undefined);
        // 실패하면 꺼짐으로 남아 다시 누를 수 있다
        expect(useUIStore.getState().backgroundDaemonStatus).toBe('stopped');
    });

    it('부르다 예외가 나도 알린다', async () => {
        window.api.daemonRestart = vi.fn().mockRejectedValue(new Error('spawn failed')) as never;
        render(<BackgroundModal onClose={vi.fn()} />);
        const button = await screen.findByRole('button', { name: i18n.t('gui:background_modal.restart_button') });
        await act(async () => fireEvent.click(button));
        expect(window.showToast).toHaveBeenCalledWith(failed('spawn failed'), 'error', 6000, undefined);
    });
});
