import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { MoreTab } from '../components/MoreTab';
import { initI18n } from '../i18n';
import { mockApi } from './mocks';

beforeAll(() => initI18n('en'));
afterEach(cleanup);

const line = (id: number, message: string) => ({ id, timestamp: Date.now(), level: 'info', target: 'x', message });

describe('웹 데몬 로그', () => {
    it('처음 읽은 뒤로는 스트림으로 새 줄을 받고, 데몬이 다시 켜져 처음부터 오면 받은 줄을 바꾼다', async () => {
        const daemonConsole = vi.fn().mockResolvedValue({ epoch: 5, lines: [line(3, 'first')] });
        let update: (body: unknown) => void = () => {};
        const close = vi.fn();
        const daemonConsoleStream = vi.fn(async (_opts: object, onUpdate: (body: unknown) => void) => {
            update = onUpdate;
            return close;
        });
        const api = mockApi({ daemonConsole, daemonConsoleStream, systemMetrics: vi.fn().mockResolvedValue(null) });
        const view = render(<MoreTab api={api} />);
        await act(async () => fireEvent.click(screen.getByText('Daemon log')));
        await act(async () => {});
        expect(daemonConsoleStream).toHaveBeenCalledWith({ since: 4, epoch: 5 }, expect.any(Function), expect.any(Function));

        await act(async () => update({ epoch: 5, lines: [line(4, 'second')] }));
        expect(screen.queryByText('second')).not.toBeNull();
        await act(async () => update({ epoch: 6, reset: true, lines: [line(1, 'again')] }));
        expect(screen.queryByText('first')).toBeNull();
        expect(screen.queryByText('again')).not.toBeNull();
        view.unmount();
        expect(close).toHaveBeenCalledTimes(1);
        expect(daemonConsole).toHaveBeenCalledTimes(1);
    });
});
