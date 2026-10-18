import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { OpenInTerminalButton, resetOpenInTerminal } from '../components/servers/OpenInTerminal';

const flush = () => act(async () => {});
const server = { id: 'inst-1', name: 'Survival' };

describe('터미널로 열기', () => {
    afterEach(() => {
        delete window.api.openInTerminal;
        window.showToast = undefined;
        resetOpenInTerminal();
    });

    it('채널이 없으면 버튼을 그리지 않는다', () => {
        window.api.openInTerminal = undefined;
        const { container } = render(<OpenInTerminalButton server={server} />);
        expect(container.innerHTML).toBe('');
    });

    it('그 서버 id로 콘솔을 열고, 성공하면 조용하다', async () => {
        window.showToast = vi.fn();
        window.api.openInTerminal = vi.fn().mockResolvedValue({ success: true, pid: 10 });
        render(<OpenInTerminalButton server={server} />);
        fireEvent.click(screen.getByRole('button', { name: 'Open in terminal' }));
        await flush();
        expect(window.api.openInTerminal).toHaveBeenCalledWith({ view: 'console', instance: 'inst-1' });
        expect(window.showToast).not.toHaveBeenCalled();
    });

    it('이 PC에서 못 열면 알리고 버튼을 숨긴다', async () => {
        window.showToast = vi.fn();
        window.api.openInTerminal = vi.fn().mockResolvedValue({ success: false, error_code: 'cli_not_found' });
        render(<OpenInTerminalButton server={server} />);
        fireEvent.click(screen.getByRole('button', { name: 'Open in terminal' }));
        await flush();
        expect(window.showToast).toHaveBeenCalledWith(
            "Couldn't find the CLI program, so the terminal can't be opened.",
            'warning',
            6000,
            undefined,
        );
        expect(screen.queryByRole('button', { name: 'Open in terminal' })).toBeNull();
    });

    it('그 밖의 오류는 알리기만 하고 버튼은 남긴다', async () => {
        window.showToast = vi.fn();
        window.api.openInTerminal = vi.fn().mockResolvedValue({ error_code: 'invalid_input', field: 'view' });
        render(<OpenInTerminalButton server={server} />);
        fireEvent.click(screen.getByRole('button', { name: 'Open in terminal' }));
        await flush();
        expect(window.showToast).toHaveBeenCalledWith("Couldn't open the terminal.", 'warning', 6000, undefined);
        expect(screen.getByRole('button', { name: 'Open in terminal' })).toBeInTheDocument();
    });
});
