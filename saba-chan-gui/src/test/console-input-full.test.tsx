import { act, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import CommandModalImpl from '../components/Modals/CommandModal';
import { useMultiConsole } from '../hooks/useMultiConsole';
import i18n from '../i18n';
import { useConsoleStore } from '../stores/useConsoleStore';
import { useUIStore } from '../stores/useUIStore';
import type { ComponentType } from 'react';

/** 시험에 필요한 props만 넘기려고 느슨하게 받는다 */
const CommandModal = CommandModalImpl as unknown as ComponentType<Record<string, unknown>>;

// 데몬이 서버 입력 대기열이 찼다고 하면(console_input_full) 영문 원인 대신 다시 보내라는 안내를 보인다
const full = { error: 'stdin command failed: console input queue is full', error_code: 'console_input_full' };

afterEach(() => {
    useConsoleStore.setState({ consoles: {} });
    delete (window.api as Partial<typeof window.api>).managedStdin;
    delete (window.api as Partial<typeof window.api>).executeCommand;
});

describe('콘솔 입력 대기열이 찼을 때', () => {
    it('콘솔은 명령 API로 다시 보내지 않고 안내한다', async () => {
        window.showToast = vi.fn();
        window.api.managedStdin = vi.fn().mockResolvedValue(full) as never;
        window.api.executeCommand = vi.fn().mockResolvedValue(full) as never;
        const { result } = renderHook(() =>
            useMultiConsole({ isPopoutMode: false, popoutParams: null, consoleBufferRef: { current: 2000 } }),
        );
        act(() => result.current.openConsole('f', 'F'));
        act(() => result.current.setConsoleInput('f', 'say hi'));
        await act(async () => result.current.sendConsoleCommand('f'));
        expect(window.api.executeCommand).not.toHaveBeenCalled();
        expect(vi.mocked(window.showToast).mock.calls.map((c) => c[0])).toEqual([i18n.t('errors.console_input_full')]);
        act(() => result.current.closeConsole('f'));
    });

    it('명령 창도 안내 문구를 보인다', async () => {
        window.api.executeCommand = vi.fn().mockResolvedValue(full) as never;
        const server = { id: 's1', name: 'S', module: 'm', status: 'running' };
        render(<CommandModal server={server} modules={[]} onClose={() => {}} />);
        const input = screen.getByRole('textbox');
        fireEvent.change(input, { target: { value: 'say hi' } });
        await act(async () => fireEvent.keyDown(input, { key: 'Enter' }));
        const entry = useUIStore.getState().commandHistoryMap.s1.at(-1) as { message: string; detail: string };
        expect(entry.message).toBe(i18n.t('errors.console_input_full'));
        expect(entry.detail).not.toMatch(/queue is full/);
    });
});
