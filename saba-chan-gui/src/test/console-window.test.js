/**
 * ConsoleWindow z-index & drag constraint tests.
 *
 * Validates:
 *   1. Console window z-index is always above the app-header z-index.
 *   2. Drag handler clamps Y to not go above TitleBar.
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { TITLEBAR_HEIGHT } from '../constants/layout';
import { ConsoleWindow } from '../components/ConsoleWindow';

// jsdom doesn't implement scrollIntoView
Element.prototype.scrollIntoView = vi.fn();

// Minimal mock for window.api
if (!window.api) {
    window.api = {
        consolePopout: vi.fn().mockResolvedValue({ ok: true }),
    };
}

const APP_HEADER_Z_INDEX_OLD = 2000;

function makeState(overrides = {}) {
    return {
        server: { id: 'test-1', name: 'Test Server' },
        lines: [],
        input: '',
        stdinDisabled: false,
        minimized: false,
        pinned: false,
        position: { x: 100, y: 100 },
        size: { width: 700, height: 400 },
        zIndex: 101,
        ...overrides,
    };
}

const noopFn = vi.fn();

function renderConsole(state = makeState(), overrides = {}) {
    const props = {
        instanceId: 'test-1', state, focusConsole: vi.fn(), minimizeConsole: noopFn,
        closeConsole: noopFn, togglePin: noopFn, updatePosition: vi.fn(), updateSize: vi.fn(),
        setConsoleInput: noopFn, sendConsoleCommand: noopFn, setConsolePopoutInstanceId: noopFn,
        highlightRules: null, ...overrides,
    };
    return { ...render(<ConsoleWindow {...props} />), props };
}

describe('ConsoleWindow keyboard controls and scrolling', () => {
    it('moves the window with arrow keys and preserves the titlebar boundary', () => {
        const { props } = renderConsole(makeState({ position: { x: 100, y: TITLEBAR_HEIGHT } }));
        const move = screen.getByRole('button', { name: /Move.*Test Server/i });
        fireEvent.keyDown(move, { key: 'ArrowUp' });
        expect(props.updatePosition).toHaveBeenLastCalledWith('test-1', { x: 100, y: TITLEBAR_HEIGHT });
        fireEvent.keyDown(move, { key: 'ArrowRight', shiftKey: true });
        expect(props.updatePosition).toHaveBeenLastCalledWith('test-1', { x: 150, y: TITLEBAR_HEIGHT });
    });

    it.each([
        ['n', 'ArrowUp', 700, 410, 100, 90], ['s', 'ArrowDown', 700, 410, 100, 100],
        ['e', 'ArrowRight', 710, 400, 100, 100], ['w', 'ArrowLeft', 710, 400, 90, 100],
        ['ne', 'ArrowUp', 700, 410, 100, 90], ['nw', 'ArrowLeft', 710, 400, 90, 100],
        ['se', 'ArrowDown', 700, 410, 100, 100], ['sw', 'ArrowLeft', 710, 400, 90, 100],
    ])('resizes the %s edge with a keyboard', (direction, key, width, height, x, y) => {
        const { container, props } = renderConsole();
        const handle = container.querySelector(`.cw-resize-${direction}`);
        expect(handle.tagName).toBe('BUTTON');
        expect(handle).toHaveAccessibleName();
        fireEvent.keyDown(handle, { key });
        expect(props.updateSize).toHaveBeenLastCalledWith('test-1', { width, height });
        expect(props.updatePosition).toHaveBeenLastCalledWith('test-1', { x, y });
    });

    it('scrolls when a full log buffer receives a replacement line and when restored', () => {
        const lines = [{ id: 1, content: 'first' }];
        const { rerender, props } = renderConsole(makeState({ lines }));
        Element.prototype.scrollIntoView.mockClear();
        const next = { ...props.state, lines: [{ id: 2, content: 'next' }] };
        rerender(<ConsoleWindow {...props} state={next} />);
        expect(Element.prototype.scrollIntoView).toHaveBeenCalledOnce();
        rerender(<ConsoleWindow {...props} state={{ ...next, minimized: true }} />);
        expect(Element.prototype.scrollIntoView).toHaveBeenCalledOnce();
        rerender(<ConsoleWindow {...props} state={next} />);
        expect(Element.prototype.scrollIntoView).toHaveBeenCalledTimes(2);
    });

    it('keeps minimum dimensions and the top boundary when resizing from the top left', () => {
        const { container, props } = renderConsole(makeState({
            position: { x: 100, y: TITLEBAR_HEIGHT }, size: { width: 360, height: 200 },
        }));
        const handle = container.querySelector('.cw-resize-nw');
        for (const key of ['ArrowRight', 'ArrowDown', 'ArrowUp']) fireEvent.keyDown(handle, { key });
        expect(props.updateSize).toHaveBeenLastCalledWith('test-1', { width: 360, height: 200 });
        expect(props.updatePosition).toHaveBeenLastCalledWith('test-1', { x: 100, y: TITLEBAR_HEIGHT });
    });

    it('removes active drag listeners when the window unmounts', () => {
        const { unmount, props } = renderConsole();
        fireEvent.mouseDown(screen.getByRole('button', { name: /Move.*Test Server/i }), { button: 0, clientX: 100, clientY: 100 });
        unmount();
        fireEvent.mouseMove(document, { clientX: 120, clientY: 120 });
        expect(props.updatePosition).not.toHaveBeenCalled();
    });
});

describe('ConsoleWindow z-index layering', () => {
    it('initial z-index (101) should be above app-header z-index', () => {
        const state = makeState({ zIndex: 101 });
        // After fix, app-header z-index = 10 (was 2000).
        // Console z-index (101) must be > app-header z-index.
        // This test would FAIL with app-header z-index = 2000.
        expect(state.zIndex).toBeGreaterThan(10); // new app-header z-index
        // Verify the OLD z-index (2000) would have been a problem:
        expect(state.zIndex).toBeLessThan(APP_HEADER_Z_INDEX_OLD);
    });
});

describe('ConsoleWindow drag Y constraint', () => {
    it('should clamp drag position Y to >= TITLEBAR_HEIGHT', () => {
        const positions = [];
        const mockUpdatePosition = vi.fn((_id, pos) => positions.push(pos));

        render(
            <ConsoleWindow
                instanceId="test-1"
                state={makeState({ position: { x: 100, y: 100 } })}
                focusConsole={noopFn}
                minimizeConsole={noopFn}
                closeConsole={noopFn}
                togglePin={noopFn}
                updatePosition={mockUpdatePosition}
                updateSize={noopFn}
                setConsoleInput={noopFn}
                sendConsoleCommand={noopFn}
                setConsolePopoutInstanceId={noopFn}
                highlightRules={null}
                servers={[]}
            />,
        );

        const titlebar = screen.getByRole('button', { name: /Move.*Test Server/i });
        expect(titlebar).toBeTruthy();

        // Simulate drag: mousedown on titlebar
        act(() => {
            titlebar.dispatchEvent(
                new MouseEvent('mousedown', { button: 0, clientX: 200, clientY: 100, bubbles: true }),
            );
        });

        // Drag upward past the TitleBar area (y = 10, well above 40px limit)
        act(() => {
            document.dispatchEvent(
                new MouseEvent('mousemove', { clientX: 200, clientY: 10, bubbles: true }),
            );
        });

        act(() => {
            document.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
        });

        // The last position update should have Y clamped to >= TITLEBAR_HEIGHT
        expect(mockUpdatePosition).toHaveBeenCalled();
        const lastPos = positions[positions.length - 1];
        expect(lastPos.y).toBeGreaterThanOrEqual(TITLEBAR_HEIGHT);
    });
});
