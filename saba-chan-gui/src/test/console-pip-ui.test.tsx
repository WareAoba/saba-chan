/**
 * PIP 개선: 창 버튼의 복원 아이콘, PIP로 띄운 동안의 안내, 콘솔 반투명 설정
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ServerConsole } from '../components/servers/ServerConsole';
import { WindowControls } from '../components/shell/AppBar';
import '../i18n';
import { useConsoleStore } from '../stores/useConsoleStore';
import type { Server } from '../stores/useServerStore';

describe('창 버튼', () => {
    afterEach(() => {
        delete (window.api as unknown as Record<string, unknown>).windowIsMaximized;
    });

    it('최대화된 창이면 가운데 버튼이 이전 크기로 돌린다', async () => {
        let maximized = true;
        Object.assign(window.api, { windowIsMaximized: vi.fn(async () => maximized) });
        render(<WindowControls />);
        await waitFor(() => expect(screen.getByRole('button', { name: 'Restore' })).toBeInTheDocument());
        maximized = false;
        act(() => {
            window.dispatchEvent(new Event('resize'));
        });
        await waitFor(() => expect(screen.getByRole('button', { name: 'Maximize' })).toBeInTheDocument());
    });
});

describe('PIP로 띄운 동안의 콘솔 자리', () => {
    afterEach(() => useConsoleStore.setState({ ghosts: {} }));

    it('마지막 줄을 흐리게 깔고, PIP 창 보기·여기로 가져오기를 보인다', () => {
        useConsoleStore.setState({ ghosts: { m: ['Done (3.2s)!', 'There are 0 players'] } });
        const onPopout = vi.fn();
        const onPopin = vi.fn();
        render(
            <ServerConsole
                server={{ id: 'm', name: 'MC', status: 'running' } as Server}
                poppedOut
                onPopout={onPopout}
                onPopin={onPopin}
                openConsole={() => {}}
                setConsoleInput={() => {}}
                sendConsoleCommand={() => {}}
            />,
        );
        const ghost = document.querySelector('.term-popped-ghost');
        expect(ghost?.getAttribute('aria-hidden')).toBe('true');
        expect(ghost?.textContent).toContain('There are 0 players');
        expect(screen.getByRole('status').textContent).toContain('Viewing in PIP window');
        fireEvent.click(screen.getByRole('button', { name: /Show PIP window/ }));
        fireEvent.click(screen.getByRole('button', { name: 'Bring it back here' }));
        expect(onPopout).toHaveBeenCalledOnce();
        expect(onPopin).toHaveBeenCalledOnce();
    });
});

describe('콘솔 반투명 설정', () => {
    it('기본은 켜짐이다', async () => {
        const { useSettingsStore } = await import('../stores/useSettingsStore');
        expect(useSettingsStore.getState().consoleGlass).toBe(true);
    });
});
