import { act, renderHook } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { useData } from '../store';
import { asServers } from './mocks';

// 4초마다 받는 서버 목록이 내용이 같아도 새 객체라, 바뀐 게 없어도 서버 화면 전체를 다시 그렸다
describe('서버 목록 다시 받기', () => {
    const list = (status = 'stopped') =>
        asServers([
            { id: 'a', name: 'Alpha', status },
            { id: 'b', name: 'Beta', status: 'stopped' },
        ]);

    it('내용이 그대로면 화면을 다시 그리지 않고, 바뀐 서버만 새로 그린다', () => {
        useData.setState({ servers: list(), serversLoaded: true });
        let renders = 0;
        const { result } = renderHook(() => {
            renders += 1;
            return useData((s) => s.servers);
        });
        const [alpha, beta] = result.current;

        act(() => useData.getState().setServers(list()));
        act(() => useData.getState().setServers(list()));
        expect(renders).toBe(1);

        act(() => useData.getState().setServers(list('running')));
        expect(renders).toBe(2);
        expect(result.current[0]).not.toBe(alpha);
        expect(result.current[0].status).toBe('running');
        expect(result.current[1]).toBe(beta);
    });
});
