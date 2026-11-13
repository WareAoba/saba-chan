import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import CommandModalImpl from '../components/Modals/CommandModal';
import { useUIStore } from '../stores/useUIStore';
import type { ComponentType } from 'react';

const CommandModal = CommandModalImpl as unknown as ComponentType<Record<string, unknown>>;
const server = { id: 'm1', name: 'M', module: 'minecraft', status: 'running' };

afterEach(() => {
    useUIStore.setState({ commandHistoryMap: {} });
    delete (window.api as Partial<typeof window.api>).executeCommand;
});

// 서버가 거절한 명령(데몬이 모듈의 실패 모양으로 판정한 `rejected`)은 기록에 실패로 남고, 까닭을 보인다
it('서버가 거절한 명령은 실패로 기록한다', async () => {
    window.api.executeCommand = vi.fn().mockResolvedValue({
        success: true,
        protocol: 'stdin',
        data: { response: '[02:33:25] [Server thread/INFO]: System chat: Incorrect argument for command' },
        rejected: { reason: 'Incorrect argument for command' },
    }) as never;
    render(<CommandModal server={server} modules={[]} />);
    const input = screen.getByRole('textbox');
    fireEvent.change(input, { target: { value: 'gamerule keepInventory' } });
    await act(async () => fireEvent.keyDown(input, { key: 'Enter' }));
    const [entry] = useUIStore.getState().commandHistoryMap.m1;
    expect(entry.status).toBe('failure');
    expect(entry.message).toBe('Incorrect argument for command');
});
