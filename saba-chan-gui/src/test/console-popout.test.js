import { act, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { PopoutConsole } from '../components/servers/PopoutConsole';
import { useMultiConsole } from '../hooks/useMultiConsole';
import { useConsoleStore } from '../stores/useConsoleStore';

const server = { id: 'mc-1', name: 'Survival', module: 'minecraft', status: 'running' };
const state = { lines: [{ id: 1, content: 'Done (3.2s)!', source: 'stdout' }], input: '' };

describe('PopoutConsole', () => {
    afterEach(() => {
        delete window.api.consolePopin;
    });

    it('상세 화면 콘솔을 그대로 쓰고, 입력칸에 커서를 두며, 다시 띄우기 버튼은 없다', () => {
        window.api.consolePopin = vi.fn(async () => ({ ok: true }));
        const setConsoleInput = vi.fn();
        render(
            <PopoutConsole
                popoutParams={{ instanceId: 'mc-1', name: 'Survival' }}
                server={server}
                state={state}
                openConsole={vi.fn()}
                setConsoleInput={setConsoleInput}
                sendConsoleCommand={vi.fn()}
            />,
        );
        expect(screen.getByText('Done (3.2s)!')).toBeInTheDocument();
        const input = screen.getByRole('textbox');
        expect(document.activeElement).toBe(input);
        fireEvent.change(input, { target: { value: 'list' } });
        expect(setConsoleInput).toHaveBeenCalledWith('mc-1', 'list');
        expect(document.querySelector('.term-popout')).toBeNull();

        fireEvent.click(document.querySelector('.console-popout-titlebar-btn'));
        expect(window.api.consolePopin).toHaveBeenCalledWith('mc-1');
    });
});

describe('useMultiConsole 따로 띄운 창', () => {
    afterEach(() => useConsoleStore.setState({ consoles: {} }));

    it('여러 서버를 동시에 띄워도 각각 기억하고, 닫힌 것만 지운다', () => {
        const { result } = renderHook(() =>
            useMultiConsole({ isPopoutMode: false, popoutParams: null, consoleBufferRef: { current: 2000 } }),
        );
        const opened = window.api.onConsolePopoutOpened.mock.calls.at(-1)[0];
        const closed = window.api.onConsolePopoutClosed.mock.calls.at(-1)[0];

        act(() => result.current.openConsole('a', 'A'));
        act(() => {
            opened('a');
            opened('b');
        });
        expect(result.current.consolePopoutIds).toEqual(['a', 'b']);
        // 띄운 창이 따로 읽으니 메인 창의 콘솔은 닫힌다
        expect(useConsoleStore.getState().consoles.a).toBeUndefined();

        act(() => closed('a'));
        expect(result.current.consolePopoutIds).toEqual(['b']);
    });

    it('메인 창이 새로고침되면 이미 떠 있는 분리 창 목록을 받아 온다', async () => {
        window.api.consolePopoutList = vi.fn().mockResolvedValue(['c']);
        const { result } = renderHook(() =>
            useMultiConsole({ isPopoutMode: false, popoutParams: null, consoleBufferRef: { current: 2000 } }),
        );
        await act(async () => {});
        expect(result.current.consolePopoutIds).toEqual(['c']);
        delete window.api.consolePopoutList;
    });

    it('콘솔 입력과 새 줄은 훅을 쓰는 화면(App)을 다시 그리지 않는다', () => {
        let renders = 0;
        const { result } = renderHook(() => {
            renders++;
            return useMultiConsole({ isPopoutMode: false, popoutParams: null, consoleBufferRef: { current: 2000 } });
        });
        act(() => result.current.openConsole('d', 'D'));
        const before = renders;

        act(() => result.current.setConsoleInput('d', 'list'));

        expect(useConsoleStore.getState().consoles.d.input).toBe('list');
        expect(renders).toBe(before);
    });
});
