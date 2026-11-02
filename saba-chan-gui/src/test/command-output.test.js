import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import CommandModal from '../components/Modals/CommandModal';
import { useUIStore } from '../stores/useUIStore';

// 값으로 답하는 조회(지표·설정 보기·플레이어 목록)는 "실행됨" 대신 데몬이 정리한 결과(output)를 보인다
const server = { id: 'p1', name: 'P', module: 'palworld', status: 'running' };
const modules = [
    { name: 'palworld', commands: { fields: [{ name: 'metrics' }, { name: 'settings' }, { name: 'players' }] } },
];

afterEach(() => {
    useUIStore.setState({ commandHistoryMap: {} });
    delete window.api.executeCommand;
});

async function run(command, output) {
    window.api.executeCommand = vi.fn().mockResolvedValue({
        success: true,
        protocol: 'rest',
        data: { status: 200, url: 'http://127.0.0.1:8212/v1/api/x', method: 'GET', response: { x: 1 } },
        output,
    });
    render(<CommandModal server={server} modules={modules} />);
    const input = screen.getByRole('textbox');
    fireEvent.change(input, { target: { value: command } });
    await act(async () => fireEvent.keyDown(input, { key: 'Enter' }));
}

const field = (key, label, value) => ({ key, label, value });

describe('명령 결과 표시', () => {
    it('이름: 값 결과를 펼치지 않아도 보인다', async () => {
        await run('metrics', { fields: [field('serverfps', 'Server FPS', '59'), field('uptime', 'uptime', '7234')] });
        expect(screen.getByText('Server FPS')).toBeTruthy();
        expect(screen.getByText('59')).toBeTruthy();
        expect(screen.getByText('7234')).toBeTruthy();
        expect(screen.queryByText(/has been executed/)).toBeNull();
    });

    it('8줄이 넘으면 앞 8줄만 보이고 더 보기로 펼친다', async () => {
        const many = Array.from({ length: 11 }, (_, i) => field(`k${i}`, `Key ${i}`, `v${i}`));
        await run('settings', { fields: many });
        expect(screen.getByText('Key 7')).toBeTruthy();
        expect(screen.queryByText('Key 8')).toBeNull();
        fireEvent.click(screen.getByRole('button', { name: /3 more/ }));
        expect(screen.getByText('Key 10')).toBeTruthy();
    });

    it('목록 결과는 표로 보인다', async () => {
        await run('players', {
            table: {
                key: 'players',
                label: 'Players',
                columns: [
                    { key: 'name', label: 'Name' },
                    { key: 'level', label: 'Level' },
                ],
                rows: [['Kim', '24']],
            },
        });
        expect(screen.getByText('Players (1)')).toBeTruthy();
        expect(screen.getByRole('columnheader', { name: 'Level' })).toBeTruthy();
        expect(screen.getByRole('cell', { name: 'Kim' })).toBeTruthy();
    });

    it('글자 결과는 그대로 보인다', async () => {
        await run('metrics', { text: 'Saved' });
        expect(screen.getByText('Saved')).toBeTruthy();
    });
});
