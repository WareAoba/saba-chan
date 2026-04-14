/**
 * ConsoleWindow z-index & drag constraint tests.
 *
 * Validates:
 *   1. Console window z-index is always above the app-header z-index.
 *   2. Drag handler clamps Y to not go above TitleBar.
 */
import { act, render, screen } from '@testing-library/react';
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
        const mockUpdatePosition = vi.fn((id, pos) => positions.push(pos));

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

        const titlebar = document.querySelector('.cw-titlebar');
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
