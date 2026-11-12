import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { BackupsPanel } from '../components/BackupsPanel';
import { createWebApi } from '../api';
import { useServerControl } from '../components/useServerControl';
import { initI18n } from '../i18n';
import { useApp } from '../store';
import { asServer, mockApi } from './mocks';

// 릴레이는 2분이 지나면 504(remote_timeout)로 답하지만 PC는 그 작업(큰 백업, 준비가 긴 첫 시작)을 계속한다.
// 실패로 단정하지 않고 아직 진행 중일 수 있다고 알린다
const slow = { success: false, error_code: 'remote_timeout', error: 'timeout' };
const lastToast = () => useApp.getState().toasts.at(-1);

describe('웹에서 2분 넘게 걸리는 작업', () => {
    beforeAll(() => initI18n('en'));
    beforeEach(() => useApp.setState({ toasts: [] }));

    it('서버 켜기가 2분을 넘기면 오류가 아니라 진행 중일 수 있다고 알린다', async () => {
        const { result } = renderHook(() => useServerControl(mockApi({ instanceStart: vi.fn().mockResolvedValue(slow) })));
        await act(async () => result.current.start(asServer({ id: 'a', name: 'A' })));
        expect(lastToast()).toMatchObject({ message: expect.stringMatching(/still be working/i), tone: 'info' });
    });

    it('지우기가 릴레이의 504로 끝나도 오류가 아니라 진행 중일 수 있다고 알린다', async () => {
        const session = { request: async () => ({ status: 504, data: slow }) };
        const { result } = renderHook(() => useServerControl(createWebApi(session).api));
        let removing: Promise<unknown> = Promise.resolve();
        act(() => {
            removing = result.current.remove(asServer({ id: 'a', name: 'A' }));
        });
        await act(async () => {
            useApp.getState().confirm?.done(true);
            await removing;
        });
        expect(lastToast()).toMatchObject({ message: expect.stringMatching(/still be working/i), tone: 'info' });
    });

    it('백업이 2분을 넘기면 진행 중일 수 있다고 알리고 목록을 다시 읽는다', async () => {
        const instanceBackups = vi.fn().mockResolvedValue({ backups: [] });
        const api = mockApi({ instanceBackups, instanceBackupCreate: vi.fn().mockResolvedValue(slow) });
        render(<BackupsPanel api={api} server={{ id: 'a' }} running={false} />);
        await waitFor(() => expect(instanceBackups).toHaveBeenCalledTimes(1));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Back up now/ })));
        expect(lastToast()).toMatchObject({ message: expect.stringMatching(/still be working/i), tone: 'info' });
        expect(instanceBackups).toHaveBeenCalledTimes(2);
    });
});
