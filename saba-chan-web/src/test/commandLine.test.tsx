import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { CommandsPanel } from '../components/CommandsPanel';
import { initI18n } from '../i18n';
import { asServer, mockApi } from './mocks';

// 사용자 11-06 #4: 웹 명령 탭도 앱과 같다. 직접 치는 명령은 입력 줄에서 색으로 나누고 안내하며,
// API로 요청하는 명령은 카드를 눌러 칸이 있는 입력 창으로 보낸다
beforeAll(() => initI18n('en'));
afterEach(cleanup);

const input = (name: string, extra: object = {}) => ({ name, label: name, type: 'text', required: true, ...extra });
const mc = {
    name: 'mc',
    commands: {
        fields: [
            {
                name: 'give',
                label: 'Give Item',
                method: 'console',
                inputs: [input('player'), input('item'), input('amount', { type: 'number', min: 1, max: 64 })],
            },
            { name: 'list', label: 'Players', method: 'console', inputs: [] },
        ],
    },
};
const pal = {
    name: 'pal',
    commands: { fields: [{ name: 'announce', label: 'Announce', method: 'rest', inputs: [input('message')] }] },
};

const panel = async (moduleData: object, executeCommand = vi.fn().mockResolvedValue({ data: {} })) => {
    const api = mockApi({ instanceCommands: vi.fn().mockResolvedValue({}), executeCommand });
    const server = asServer({ id: 's1', name: 'S', module: moduleData === mc ? 'mc' : 'pal', status: 'running' });
    await act(async () => render(<CommandsPanel api={api} server={server} moduleData={moduleData} running />));
    return executeCommand;
};

describe('웹 명령 입력', () => {
    it('직접 치는 명령은 입력 줄에서 칸마다 색을 나누고 이유를 알리며, 친 줄 그대로 보낸다', async () => {
        const exec = await panel(mc);
        const line = screen.getByRole('textbox', { name: 'Command' });
        fireEvent.change(line, { target: { value: 'give Steve diamond 99' } });
        expect([...document.querySelectorAll('.cmdline-hl [class^="tok-"]')].map((e) => e.className)).toEqual([
            'tok-name',
            'tok-ok',
            'tok-ok',
            'tok-bad',
        ]);
        // 틀린 칸은 글자 기호 대신 x 아이콘(tone-bad)으로 알린다 (#944)
        const status = screen.getByRole('status');
        expect(status.textContent).toBe('amount allows only 1–64. Got: 99');
        expect(status.classList.contains('tone-bad')).toBe(true);
        fireEvent.change(line, { target: { value: 'give Steve diamond 3' } });
        await act(async () => fireEvent.submit(line.closest('form') as HTMLFormElement));
        expect(exec).toHaveBeenCalledWith('s1', { command: 'give Steve diamond 3' });
    });

    it('칸이 있는 직접 명령 카드를 누르면 입력 줄에 이름을 넣는다', async () => {
        const exec = await panel(mc);
        fireEvent.click(screen.getByRole('button', { name: /Give Item/ }));
        expect((screen.getByRole('textbox', { name: 'Command' }) as HTMLInputElement).value).toBe('give ');
        expect(exec).not.toHaveBeenCalled();
    });

    it('API로 요청하는 명령만 있는 모듈은 입력 줄 없이 카드와 입력 창으로 둔다', async () => {
        const exec = await panel(pal);
        expect(screen.queryByRole('textbox', { name: 'Command' })).toBeNull();
        fireEvent.click(screen.getByRole('button', { name: /Announce/ }));
        fireEvent.change(screen.getByLabelText(/message/), { target: { value: 'hello' } });
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Send' })));
        expect(exec).toHaveBeenCalledWith('s1', { command: 'announce', args: { message: 'hello' } });
    });
});
