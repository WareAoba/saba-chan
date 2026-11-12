import { act, fireEvent, render, screen } from '@testing-library/react';
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
const minecraft = {
    name: 'minecraft',
    game_name: 'Minecraft',
    protocols: { supported: ['stdin'] },
    commands: {
        fields: [
            { name: 'list', label: 'List', method: 'console' },
            { name: 'say', label: 'Say', method: 'console', inputs: [{ name: 'message' }] },
            { name: 'stop', label: 'Stop', description: 'Saves the world and stops the server', method: 'console', risk: 'danger' },
            { name: 'wipe', label: 'Wipe', method: 'console', risk: 'danger' },
            { name: 'players', label: 'Players', method: 'rest' },
        ],
    },
};
const LINES = [
    { id: 0, timestamp: 1_700_000_000, level: 'INFO', content: 'mango joined the game', spans: [[0, 5, 'player']] },
    { id: 1, timestamp: 1_700_000_001, level: 'WARN', content: 'slow tick' },
    { id: 2, timestamp: 1_700_000_002, level: 'ERROR', content: 'chunk lost' },
];

/** 켜진 서버의 콘솔을 열고 첫 읽기를 마친다 */
async function open(own: Record<string, unknown> = {}) {
    const managedConsole = vi.fn().mockImplementation(async (_id: string, since: number) => ({
        running: true,
        epoch: 1,
        stdin_available: true,
        lines: LINES.filter((l) => l.id >= since),
    }));
    const api = { managedConsole, managedStdin: vi.fn().mockResolvedValue({ success: true }), ...own };
    const server = asServer({ id: 's1', name: 'MC', module: 'minecraft', status: 'running' });
    let container!: HTMLElement;
    await act(async () => {
        container = render(<ServerDetail api={apiWith(api)} server={server} />).container;
    });
    const lines = () => [...container.querySelectorAll('.console-lines .line')];
    return { api, container, lines };
}

describe('웹 콘솔 표시 (앱 콘솔과 같은 모습)', () => {
    beforeAll(() => initI18n('en'));
    beforeEach(() => {
        vi.useFakeTimers();
        useData.setState({ modules: asModules([minecraft]), modulesLoaded: true });
    });
    afterEach(() => vi.useRealTimers());

    it('줄 앞에 시각이 붙고, 데몬이 나눈 조각대로 색을 입힌다', async () => {
        const { api, lines } = await open();
        expect(api.managedConsole).toHaveBeenCalledWith('s1', 0, 200, { spans: true });
        const first = lines()[0];
        expect(first.querySelector('.console-time')?.textContent).toMatch(/^\d{2}:\d{2}:\d{2}$/);
        expect(first.querySelector('.tok-player')?.textContent).toBe('mango');
        expect(first.textContent).toContain('mango joined the game');
    });

    it('수준 칩에 개수가 보이고, 누르면 그 수준의 줄만 남는다', async () => {
        const { lines } = await open();
        const errors = screen.getByRole('button', { name: 'Errors 1' });
        expect(screen.getByRole('button', { name: 'Warnings 1' })).toBeTruthy();
        fireEvent.click(errors);
        expect(lines().map((l) => l.textContent)).toEqual([expect.stringContaining('chunk lost')]);
        fireEvent.click(screen.getByRole('button', { name: 'All' }));
        expect(lines()).toHaveLength(3);
    });

    it('빠른 명령 칩은 콘솔로 보낼 수 있는 명령만 보이고, 누르면 입력 칸을 채운다', async () => {
        const { api } = await open();
        const quick = screen.getByRole('group', { name: 'Quick commands' });
        // 칩 글자는 누르면 입력 칸에 들어가는 명령 이름이다
        expect([...quick.querySelectorAll('button')].map((b) => b.textContent)).toEqual(['list', 'say …', 'stop', 'wipe']);
        fireEvent.click(screen.getByRole('button', { name: 'say …' }));
        expect((screen.getByPlaceholderText('Type a command') as HTMLInputElement).value).toBe('say ');
        expect(api.managedStdin).not.toHaveBeenCalled();
    });

    it('위험한 명령은 한 번 더 눌러야 보낸다', async () => {
        const { api, container } = await open();
        const input = screen.getByPlaceholderText('Type a command');
        const form = container.querySelector('.console-input') as HTMLFormElement;
        fireEvent.change(input, { target: { value: 'stop' } });
        await act(async () => fireEvent.submit(form));
        expect(api.managedStdin).not.toHaveBeenCalled();
        // 모듈이 준 명령 설명이 알림 앞에 온다
        expect(screen.getByRole('alert').textContent).toMatch(/^Saves the world and stops the server\. .*once more/i);
        await act(async () => fireEvent.submit(form));
        expect(api.managedStdin).toHaveBeenCalledWith('s1', 'stop');
        expect(screen.queryByRole('alert')).toBeNull();
    });

    it('설명이 없는 위험한 명령은 공통 문구로 알린다', async () => {
        const { container } = await open();
        fireEvent.change(screen.getByPlaceholderText('Type a command'), { target: { value: 'wipe' } });
        await act(async () => fireEvent.submit(container.querySelector('.console-input') as HTMLFormElement));
        expect(screen.getByRole('alert').textContent).toMatch(/wipe.*once more/i);
    });

    it('게임이 줄 앞에 찍은 시각(모듈이 timestamp로 선언한 조각)은 데몬 시각을 붙인 줄에서 뺀다', async () => {
        const content = '[12:00:00] [Server thread/INFO]: Done';
        const managedConsole = vi.fn().mockResolvedValue({
            running: true,
            epoch: 1,
            stdin_available: true,
            lines: [
                { id: 0, timestamp: 1_700_000_000, level: 'INFO', content, spans: [[0, 10, 'timestamp'], [11, 31, 'dim']] },
                // 데몬 시각이 없는 줄(디스크에서 읽은 옛 줄)은 게임이 찍은 시각을 그대로 둔다
                { id: 1, level: 'INFO', content, spans: [[0, 10, 'timestamp']] },
                // 조각이 괄호를 닫지 않으면(`[시각 수준]` 꼴) 빼지 않는다: 줄이 깨진다
                { id: 2, timestamp: 1_700_000_000, level: 'INFO', content: '[12:00:00 INFO]: Done', spans: [[0, 9, 'timestamp']] },
            ],
        });
        const { lines } = await open({ managedConsole });
        const [stamped, bare, joined] = lines();
        expect(stamped.querySelector('.console-time')?.textContent).toMatch(/^\d{2}:\d{2}:\d{2}$/);
        expect(stamped.querySelector('.tok-timestamp')).toBeNull();
        expect(stamped.textContent).toMatch(/^\d{2}:\d{2}:\d{2}\[Server thread\/INFO\]: Done$/);
        expect(stamped.querySelector('.tok-dim')?.textContent).toBe('[Server thread/INFO]');
        expect(bare.textContent).toBe(content);
        expect(joined.textContent).toContain('[12:00:00 INFO]: Done');
    });

    it('↑ 로 보냈던 명령을 다시 꺼낸다', async () => {
        const { container } = await open();
        const input = screen.getByPlaceholderText('Type a command') as HTMLInputElement;
        fireEvent.change(input, { target: { value: 'list' } });
        await act(async () => fireEvent.submit(container.querySelector('.console-input') as HTMLFormElement));
        expect(input.value).toBe('');
        fireEvent.keyDown(input, { key: 'ArrowUp' });
        expect(input.value).toBe('list');
        fireEvent.keyDown(input, { key: 'ArrowDown' });
        expect(input.value).toBe('');
    });
});
