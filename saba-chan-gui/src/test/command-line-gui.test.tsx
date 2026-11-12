/**
 * #4: 직접 치는 게임(콘솔로 가는 명령)은 입력 줄을 색으로 나누고 무엇이 맞고 틀렸는지 아래에 알린다.
 * API로 요청하는 명령(rest·module)은 누르면 칸이 있는 입력 창이 뜬다. 칸 정보가 없는 모듈은 평범한 입력 줄이다.
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import type { ComponentType } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import CommandModalImpl from '../components/Modals/CommandModal';
import '../i18n';
import { useUIStore } from '../stores/useUIStore';

const CommandModal = CommandModalImpl as unknown as ComponentType<Record<string, unknown>>;

const input = (name: string, extra: object = {}) => ({ name, label: name, type: 'text', required: true, ...extra });
const modules = [
    {
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
    },
    {
        name: 'pal',
        commands: {
            fields: [
                {
                    name: 'announce',
                    label: 'Announce',
                    method: 'rest',
                    inputs: [input('message', { label: 'Message' })],
                },
            ],
        },
    },
    { name: 'bare', commands: { fields: [] } },
];

const open = (module: string) =>
    render(<CommandModal server={{ id: 's1', name: 'S', module, status: 'running' }} modules={modules} />);

afterEach(() => {
    useUIStore.setState({ commandHistoryMap: {} });
    delete (window.api as Partial<typeof window.api>).executeCommand;
});

describe('명령 입력 안내', () => {
    it('직접 치는 명령은 칸마다 색을 나누고, 틀린 칸의 이유를 알린다', () => {
        open('mc');
        fireEvent.change(screen.getByRole('textbox'), { target: { value: 'give Steve diamond 99' } });
        expect([...document.querySelectorAll('.cmdline-hl [class^="tok-"]')].map((e) => e.className)).toEqual([
            'tok-name',
            'tok-ok',
            'tok-ok',
            'tok-bad',
        ]);
        expect(screen.getByRole('status').textContent).toBe('amount takes 1–64. Got: 99');
    });

    it('빠진 칸을 알리고, 다 채우면 보낼 수 있다고 알린다', () => {
        open('mc');
        const line = screen.getByRole('textbox');
        fireEvent.change(line, { target: { value: 'give Steve ' } });
        expect(screen.getByRole('status').textContent).toBe('Add item too.');
        fireEvent.change(line, { target: { value: 'give Steve diamond 3' } });
        expect(screen.getByRole('status').textContent).toBe('Ready. Press Enter to send.');
    });

    // WebKit(맥·리눅스 웹뷰)은 변환을 확정하는 Enter를 조합이 끝난 뒤 keyCode 229로 보낸다
    it('IME 변환을 확정하는 Enter는 명령을 보내지 않는다', async () => {
        window.api.executeCommand = vi.fn().mockResolvedValue({ data: {} }) as never;
        open('mc');
        const line = screen.getByRole('textbox');
        fireEvent.change(line, { target: { value: 'say こんにちは' } });
        await act(async () => fireEvent.keyDown(line, { key: 'Enter', keyCode: 229 }));
        expect(window.api.executeCommand).not.toHaveBeenCalled();
        await act(async () => fireEvent.keyDown(line, { key: 'Enter', keyCode: 13 }));
        expect(window.api.executeCommand).toHaveBeenCalledTimes(1);
    });

    it('목록에 없는 명령도 막지 않고 그대로 보낸다', async () => {
        window.api.executeCommand = vi.fn().mockResolvedValue({ data: {} }) as never;
        open('mc');
        const line = screen.getByRole('textbox');
        fireEvent.change(line, { target: { value: 'tp Steve Alex' } });
        expect(screen.getByRole('status').textContent).toMatch(/sent to the server exactly as typed/);
        await act(async () => fireEvent.keyDown(line, { key: 'Enter' }));
        expect(window.api.executeCommand).toHaveBeenCalledWith('s1', { command: 'tp Steve Alex' });
    });

    it('API로 요청하는 명령은 누르면 칸이 있는 입력 창이 뜨고, 칸 값을 이름 붙여 보낸다', async () => {
        window.api.executeCommand = vi.fn().mockResolvedValue({ data: {} }) as never;
        open('pal');
        fireEvent.click(screen.getByRole('option', { name: /Announce/ }));
        const send = screen.getByRole('button', { name: 'Execute' });
        expect((send as HTMLButtonElement).disabled).toBe(true);
        fireEvent.change(screen.getByLabelText(/Message/), { target: { value: 'hello all' } });
        await act(async () => fireEvent.click(send));
        expect(window.api.executeCommand).toHaveBeenCalledWith('s1', {
            command: 'announce',
            args: { message: 'hello all' },
        });
        expect(useUIStore.getState().commandHistoryMap.s1[0].command).toBe('announce hello all');
    });

    it('칸 정보가 없는 모듈은 평범한 입력 줄로 둔다 (안내 없음)', () => {
        open('bare');
        fireEvent.change(screen.getByRole('textbox'), { target: { value: 'anything goes' } });
        expect(screen.queryByRole('status')).toBeNull();
    });
});
