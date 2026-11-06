import { act, renderHook } from '@testing-library/react';
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerControl } from '../components/useServerControl';
import { initI18n } from '../i18n';
import { useApp } from '../store';
import { asServer, mockApi } from './mocks';

// 0.1.13 감사 #4: 이미 켜진 서버를 켜도 "켰어요", 켜는 동안 정지해도 오류로 보였다. GUI와 같이 나눈다.
describe('웹 서버 켜기 알림', () => {
    beforeAll(() => initI18n('en'));
    beforeEach(() => useApp.setState({ toasts: [] }));

    const start = async (res: object) => {
        const { result } = renderHook(() => useServerControl(mockApi({ instanceStart: vi.fn().mockResolvedValue(res) })));
        await act(async () => result.current.start(asServer({ id: 'a', name: 'A' })));
        return useApp.getState().toasts.at(-1);
    };

    it('이미 켜져 있으면 켰다고 하지 않는다', async () => {
        expect(await start({ success: true, already_running: true })).toMatchObject({
            message: expect.stringMatching(/already running/i),
            tone: 'info',
        });
    });

    it('켜는 동안 정지했으면 오류가 아니라 안내로 보인다', async () => {
        expect(await start({ success: false, error_code: 'start_cancelled', error: 'x' })).toMatchObject({
            message: expect.stringMatching(/stopped while it was starting/),
            tone: 'info',
        });
    });

    it('그냥 켜졌으면 켰다고 한다', async () => {
        expect((await start({ success: true }))?.message).toBe('Started A.');
    });
});
