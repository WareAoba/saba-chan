import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PrivacyErase } from '../components/discord/PrivacyErase';

describe('개인정보 모두 지우기', () => {
    beforeEach(() => {
        vi.useFakeTimers();
        window.showToast = vi.fn();
        window.electron = { shell: { openExternal: vi.fn().mockResolvedValue(undefined) } };
        window.api = {
            relayPrivacyErase: vi.fn().mockResolvedValue({ url: 'https://saba-chan.online/legal/erase#e.s' }),
            relayPrivacyEraseStatus: vi
                .fn()
                .mockResolvedValueOnce({ status: 'pending' })
                .mockResolvedValue({ status: 'done' }),
        };
    });
    afterEach(() => vi.useRealTimers());

    it('누르면 데몬이 준 웹사이트 주소를 열고, 데몬이 끝났다고 하면 연결을 비운다', async () => {
        const onErased = vi.fn();
        render(<PrivacyErase onErased={onErased} />);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Erase all my data/i })));
        expect(window.api.relayPrivacyErase).toHaveBeenCalledWith('en');
        expect(window.electron.shell.openExternal).toHaveBeenCalledWith('https://saba-chan.online/legal/erase#e.s');
        expect(screen.getByText(/Waiting for your consent/i)).toBeInTheDocument();
        await act(async () => vi.advanceTimersByTimeAsync(3000));
        expect(onErased).not.toHaveBeenCalled();
        await act(async () => vi.advanceTimersByTimeAsync(3000));
        expect(onErased).toHaveBeenCalledTimes(1);
    });

    it('상태를 한 번 못 받아도(오류 본문) 계속 묻는다', async () => {
        window.api.relayPrivacyEraseStatus = vi
            .fn()
            .mockResolvedValueOnce({ success: false, error: 'connect ECONNREFUSED' })
            .mockResolvedValue({ status: 'done' });
        const onErased = vi.fn();
        render(<PrivacyErase onErased={onErased} />);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Erase all my data/i })));
        await act(async () => vi.advanceTimersByTimeAsync(3000));
        expect(window.showToast).not.toHaveBeenCalled();
        await act(async () => vi.advanceTimersByTimeAsync(3000));
        expect(onErased).toHaveBeenCalledTimes(1);
    });

    it('기한이 지나면 연결은 그대로 두고 다시 누를 수 있다', async () => {
        window.api.relayPrivacyEraseStatus = vi.fn().mockResolvedValue({ status: 'expired' });
        const onErased = vi.fn();
        render(<PrivacyErase onErased={onErased} />);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Erase all my data/i })));
        await act(async () => vi.advanceTimersByTimeAsync(3000));
        expect(onErased).not.toHaveBeenCalled();
        expect(window.showToast).toHaveBeenCalled();
        expect(screen.getByRole('button', { name: /Erase all my data/i })).toBeEnabled();
    });

    it('데몬이 주소를 주지 못하면 알리고 기다리지 않는다', async () => {
        window.api.relayPrivacyErase = vi.fn().mockResolvedValue({ error: 'Relay connection failed' });
        render(<PrivacyErase onErased={vi.fn()} />);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Erase all my data/i })));
        expect(window.electron.shell.openExternal).not.toHaveBeenCalled();
        expect(window.showToast).toHaveBeenCalled();
        expect(window.api.relayPrivacyEraseStatus).not.toHaveBeenCalled();
    });
});
