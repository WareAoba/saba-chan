import { act, render } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { ServerDetail } from '../components/ServerDetail';
import { initI18n } from '../i18n';
import { useData } from '../store';
import { asModules, asServer, mockApi } from './mocks';

const apiWith = (own: Record<string, unknown>) =>
    mockApi(
        new Proxy(own, {
            get: (target, prop: string) => {
                if (!(prop in target)) target[prop] = vi.fn().mockResolvedValue({});
                return target[prop];
            },
        }),
    );
const minecraft = { name: 'minecraft', game_name: 'Minecraft', protocols: { supported: ['stdin'] }, commands: { fields: [] } };

/** 콘솔을 열고 읽기 주기를 `polls`번 더 돌린 뒤 보이는 줄 */
async function shown(managedConsole: unknown, status: string, polls: number, between?: () => void) {
    const server = asServer({ id: 's1', name: 'MC', module: 'minecraft', status });
    let container!: HTMLElement;
    await act(async () => {
        container = render(<ServerDetail api={apiWith({ managedConsole })} server={server} />).container;
    });
    for (let i = 0; i < polls; i++) {
        if (i === 1) between?.();
        await act(async () => {
            await vi.advanceTimersByTimeAsync(1500);
        });
    }
    return [...container.querySelectorAll('.console-lines .line')].map((el) => el.textContent);
}

describe('웹 콘솔 읽기', () => {
    beforeAll(() => initI18n('en'));
    beforeEach(() => {
        vi.useFakeTimers();
        useData.setState({ modules: asModules([minecraft]), modulesLoaded: true });
    });
    afterEach(() => vi.useRealTimers());

    it('꺼진 서버의 지난 로그는 읽을 때마다 다시 붙지 않는다', async () => {
        // 데몬은 프로세스가 없으면 since 를 무시하고 매번 같은 디스크 꼬리를 준다
        const managedConsole = vi.fn().mockResolvedValue({
            running: false,
            source: 'disk',
            lines: [
                { id: 0, content: 'A' },
                { id: 1, content: 'B' },
            ],
        });
        expect(await shown(managedConsole, 'stopped', 3)).toEqual(['A', 'B']);
    });

    it('다시 시작한 서버의 새 줄이 이어서 보인다', async () => {
        // 새 프로세스의 버퍼는 세대(epoch)가 바뀌고 줄 번호가 0부터 다시 시작한다
        let run = 1;
        const managedConsole = vi.fn().mockImplementation(async (_id: string, since: number) => {
            const all =
                run === 1
                    ? Array.from({ length: 50 }, (_, i) => ({ id: i, content: `old${i}` }))
                    : [
                          { id: 0, content: 'new0' },
                          { id: 1, content: 'new1' },
                      ];
            return { running: true, epoch: run, stdin_available: true, lines: all.filter((l) => l.id >= since) };
        });
        const lines = await shown(managedConsole, 'running', 4, () => {
            run = 2;
        });
        expect(lines.slice(-2)).toEqual(['new0', 'new1']);
    });

    it('껐다 켜는 사이에 지난 로그를 읽어도 새 실행의 첫 줄을 놓치지 않는다', async () => {
        let phase = 0;
        const managedConsole = vi.fn().mockImplementation(async (_id: string, since: number) => {
            if (phase === 0)
                return { source: 'disk', lines: Array.from({ length: 5 }, (_, i) => ({ id: i, content: `disk${i}` })) };
            return { epoch: 7, lines: [{ id: 0, content: 'boot' }].filter((l) => l.id >= since) };
        });
        const lines = await shown(managedConsole, 'stopped', 3, () => {
            phase = 1;
        });
        expect(lines).toEqual(['disk0', 'disk1', 'disk2', 'disk3', 'disk4', 'boot']);
    });

    it('처음 읽은 뒤로는 스트림으로 받아 주기 조회를 쉬고, 프로세스에 다시 붙으면 그 버퍼로 바꾼다', async () => {
        const managedConsole = vi.fn().mockResolvedValue({ epoch: 1, stdin_available: true, lines: [{ id: 0, content: 'hello' }] });
        let update: (body: unknown) => void = () => {};
        const close = vi.fn();
        const managedConsoleStream = vi.fn(async (_id: string, _opts: object, onUpdate: (body: unknown) => void) => {
            update = onUpdate;
            return close;
        });
        const server = asServer({ id: 's1', name: 'MC', module: 'minecraft', status: 'running' });
        let view!: ReturnType<typeof render>;
        await act(async () => {
            view = render(<ServerDetail api={apiWith({ managedConsole, managedConsoleStream })} server={server} />);
        });
        expect(managedConsoleStream).toHaveBeenCalledWith('s1', expect.objectContaining({ since: 1, epoch: 1 }), expect.any(Function), expect.any(Function));
        const text = () => [...view.container.querySelectorAll('.console-lines .line')].map((el) => el.textContent);
        await act(async () => update({ epoch: 1, lines: [{ id: 1, content: 'world' }] }));
        expect(text()).toEqual(['hello', 'world']);
        await act(async () => update({ epoch: 2, reset: true, lines: [{ id: 0, content: 'again' }] }));
        expect(text()).toEqual(['again']);
        await act(async () => {
            await vi.advanceTimersByTimeAsync(10_000);
        });
        expect(managedConsole).toHaveBeenCalledTimes(1);
        view.unmount();
        expect(close).toHaveBeenCalledTimes(1);
    });
});
