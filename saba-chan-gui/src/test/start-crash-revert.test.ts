import { act } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setupServerStore } from './serverStore';
import i18n from '../i18n';

// 시작 요청은 성공했지만 프로세스가 바로 죽으면 '시작 중' 표시를 남기지 않는다
describe('시작 직후 프로세스가 죽은 서버', () => {
    let servers: Record<string, unknown>[];
    beforeEach(() => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        servers = [{ id: 'a', name: 'A', module: 'game', status: 'stopped' }];
        window.api = {
            serverList: vi
                .fn()
                .mockResolvedValue({ servers: [{ id: 'a', name: 'A', module: 'game', status: 'stopped' }] }),
            instanceStart: vi.fn().mockResolvedValue({ success: true }),
            instanceStatus: vi.fn().mockResolvedValue({ status: 'stopped' }),
        } as unknown as typeof window.api;
        window.showToast = vi.fn();
    });
    afterEach(() => vi.useRealTimers());

    it('크래시 판정 뒤 목록 새로고침은 멈춤 상태를 보여 준다', async () => {
        const { result } = setupServerStore({ servers });
        await act(async () => result.current.fetchServers());
        await act(async () => result.current.startServer('a'));
        for (let i = 0; i < 6; i++) await act(async () => vi.advanceTimersByTime(500));
        expect(vi.mocked(window.showToast!).mock.calls.at(-1)![1]).toBe('error');
        await act(async () => result.current.fetchServers());
        expect(result.current.servers[0].status).toBe('stopped');
    });

    it('바로 꺼졌다는 알림은 번역된 문구로 보인다', async () => {
        const { result } = setupServerStore({ servers });
        await act(async () => result.current.fetchServers());
        await act(async () => result.current.startServer('a'));
        for (let i = 0; i < 6; i++) await act(async () => vi.advanceTimersByTime(500));
        const message = String(vi.mocked(window.showToast!).mock.calls.at(-1)![0]);
        expect(message).not.toContain('Process exited immediately');
        expect(message).toBe(
            i18n.t('servers.start_failed_toast', { ns: 'gui', error: i18n.t('servers.start_exited_immediately', { ns: 'gui' }) }),
        );
        expect(message).not.toContain('start_exited_immediately');
    });
});
