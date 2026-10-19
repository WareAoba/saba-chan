import { act, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { LoadingScreen } from '../components/LoadingScreen';

const flush = () => act(async () => {});

describe('LoadingScreen 데몬 로그 줄', () => {
    beforeEach(() => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        window.api.daemonConsole = vi.fn().mockResolvedValue({ lines: [{ level: 'info', message: '모듈 3개 읽음' }] });
    });
    afterEach(() => {
        vi.useRealTimers();
        delete window.api.daemonConsole;
    });

    it('로딩 중에는 마지막 로그를 보이고, 화면이 사라지면 조회를 멈춘다', async () => {
        const { unmount } = render(<LoadingScreen logoSrc="" initProgress={10} />);
        await flush();
        expect(screen.getByText('모듈 3개 읽음')).toBeTruthy();
        await act(async () => vi.advanceTimersByTime(1300));
        const calls = window.api.daemonConsole.mock.calls.length;
        expect(calls).toBeGreaterThan(1);
        unmount();
        await act(async () => vi.advanceTimersByTime(5000));
        expect(window.api.daemonConsole.mock.calls.length).toBe(calls);
    });

    it('사라지는 중(leaving)이거나 실패 화면이면 조회하지 않는다', async () => {
        render(<LoadingScreen logoSrc="" initProgress={100} leaving />);
        render(<LoadingScreen logoSrc="" initProgress={0} failed="x" />);
        await act(async () => vi.advanceTimersByTime(2000));
        expect(window.api.daemonConsole).not.toHaveBeenCalled();
    });

    it('로그에 수준이 없으면 수준 칸을 비운다 (UNDEFINED를 쓰지 않는다)', async () => {
        window.api.daemonConsole = vi.fn().mockResolvedValue({ lines: [{ message: '준비 중' }] });
        const { container } = render(<LoadingScreen logoSrc="" initProgress={10} />);
        await flush();
        expect(screen.getByText('준비 중')).toBeTruthy();
        expect(container.textContent).not.toContain('UNDEFINED');
        expect(container.querySelector('.loading-log-level')).toBeNull();
    });
});
