import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { checkAvailable, ServerCheck } from '../components/servers/ServerCheck';

const server = { id: 'a', name: 'A', module: 'game', status: 'stopped' };

describe('서버 점검 (validate·diagnose)', () => {
    beforeEach(() => {
        window.api = {};
    });

    it('채널이 없으면 아무것도 그리지 않는다', () => {
        expect(checkAvailable()).toBe(false);
        const { container } = render(<ServerCheck server={server} />);
        expect(container).toBeEmptyDOMElement();
    });

    it('점검 결과의 문제와 해결 방법을 심각도 순으로 보인다', async () => {
        window.api.instanceValidate = vi.fn().mockResolvedValue({
            success: false,
            issues: [
                { code: 'W', severity: 'warning', message: 'Low memory', solution: 'Add RAM' },
                { code: 'C', severity: 'critical', message: 'Java not found', solution: 'Install Java 21' },
            ],
        });
        render(<ServerCheck server={server} />);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Check server/i })));
        expect(window.api.instanceValidate).toHaveBeenCalledWith('a');
        const items = screen.getAllByRole('listitem');
        expect(items[0].textContent).toContain('Java not found');
        expect(items[0].textContent).toContain('Install Java 21');
        expect(items[1].textContent).toContain('Low memory');
    });

    it('문제가 없으면 없다고 말하고, 실패하면 오류를 보인다', async () => {
        window.api.instanceValidate = vi.fn().mockResolvedValue({ success: true, issues: [] });
        window.api.instanceDiagnose = vi.fn().mockResolvedValue({ error: 'boom' });
        render(<ServerCheck server={server} />);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Check server/i })));
        expect(screen.getByText(/No problems found/i)).toBeInTheDocument();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Diagnose log/i })));
        expect(window.api.instanceDiagnose).toHaveBeenCalledWith('a');
        expect(screen.getByRole('alert').textContent).toContain('boom');
    });
});
