import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { OtherDaemons } from '../components/shell/OtherDaemons';

const LIST = {
    daemons: [
        { hostId: null, name: '', ok: true, servers: [{ id: 'l1', name: 'test-pal', module: 'palworld', status: 'stopped' }] },
        { hostId: 'host-b', name: '게임 PC', ok: true, servers: [{ id: 'b1', name: 'test-pal', module: 'palworld', status: 'running' }] },
        { hostId: 'host-c', name: '클라우드', ok: false, error_code: 'remote_unreachable', servers: [] },
    ],
};

describe('합쳐 보기: 다른 데몬의 서버 (G1c)', () => {
    let switched;
    beforeEach(() => {
        switched = vi.fn();
        window.showToast = vi.fn();
        window.api = {
            remoteServers: vi.fn().mockResolvedValue(LIST),
            remoteInstanceAction: vi.fn().mockResolvedValue({ success: true }),
            daemonTargetSet: vi.fn().mockResolvedValue({ success: true }),
        };
    });
    afterEach(() => {
        vi.useRealTimers();
        delete window.showToast;
    });

    it('고른 데몬은 빼고 다른 데몬마다 절을 둔다. 닿지 않는 데몬은 이유를 보인다', async () => {
        await act(async () => render(<OtherDaemons currentTarget="host-b" onSwitched={switched} />));
        expect(screen.getByRole('region', { name: 'This PC' })).toHaveTextContent('test-pal');
        expect(screen.queryByRole('region', { name: '게임 PC' })).toBeNull();
        expect(screen.getByRole('region', { name: '클라우드' })).toHaveTextContent('Could not reach the remote daemon');
    });

    it('서버 하나는 그 데몬에 바로 켜고 끄고, 끝나면 목록을 다시 읽는다', async () => {
        await act(async () => render(<OtherDaemons currentTarget={null} onSwitched={switched} />));
        const pc = screen.getByRole('region', { name: '게임 PC' });
        await act(async () => fireEvent.click(within(pc).getByRole('button', { name: 'test-pal Stop' })));
        expect(window.api.remoteInstanceAction).toHaveBeenCalledWith('host-b', 'b1', 'stop');
        expect(window.api.remoteServers).toHaveBeenCalledTimes(2);
    });

    it('켜지 못하면 데몬이 준 문구를 알린다', async () => {
        window.api.remoteInstanceAction.mockResolvedValue({ success: false, error: '동의가 필요해요' });
        await act(async () => render(<OtherDaemons currentTarget="host-b" onSwitched={switched} />));
        const pc = screen.getByRole('region', { name: 'This PC' });
        await act(async () => fireEvent.click(within(pc).getByRole('button', { name: 'test-pal Start' })));
        expect(window.api.remoteInstanceAction).toHaveBeenCalledWith(null, 'l1', 'start');
        expect(window.showToast.mock.calls[0].slice(0, 3)).toEqual(['동의가 필요해요', 'error', 5000]);
    });

    it('"이 데몬 열기"는 그 데몬으로 바꾸고 화면을 다시 읽는다', async () => {
        await act(async () => render(<OtherDaemons currentTarget="host-b" onSwitched={switched} />));
        const pc = screen.getByRole('region', { name: 'This PC' });
        await act(async () => fireEvent.click(within(pc).getByRole('button', { name: 'Open this daemon' })));
        expect(window.api.daemonTargetSet).toHaveBeenCalledWith(null);
        expect(switched).toHaveBeenCalled();
    });

    it('10초마다 다시 읽는다', async () => {
        vi.useFakeTimers();
        await act(async () => render(<OtherDaemons currentTarget={null} onSwitched={switched} />));
        expect(window.api.remoteServers).toHaveBeenCalledTimes(1);
        await act(async () => vi.advanceTimersByTime(10_000));
        expect(window.api.remoteServers).toHaveBeenCalledTimes(2);
    });
});
