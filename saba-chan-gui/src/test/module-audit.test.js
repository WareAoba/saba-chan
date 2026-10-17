import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
    clearModuleAuditCache,
    ExtensionAuditNotice,
    ModuleAuditNotice,
} from '../components/servers/ModuleAuditNotice';

describe('ModuleAuditNotice (데몬 M12)', () => {
    afterEach(() => {
        delete window.api.moduleAudit;
        delete window.api.extensionAudit;
        clearModuleAuditCache();
    });

    it('연결 함수가 없거나 선언 밖 사용이 없으면 아무것도 안 그린다', async () => {
        const { container, rerender } = render(<ModuleAuditNotice moduleName="game" />);
        expect(container.innerHTML).toBe('');
        window.api.moduleAudit = vi.fn().mockResolvedValue({ declared: {}, findings: [], undeclared: [] });
        rerender(<ModuleAuditNotice moduleName="other" />);
        await waitFor(() => expect(window.api.moduleAudit).toHaveBeenCalledWith('other'));
        expect(container.innerHTML).toBe('');
    });

    it('선언하지 않은 능력을 경고하고, 펼치면 파일:줄을 보여 준다', async () => {
        window.api.moduleAudit = vi.fn().mockResolvedValue({
            undeclared: [
                { permission: 'network', file: 'lifecycle.py', line: 12, detail: 'urllib.request' },
                { permission: 'network', file: 'lifecycle.py', line: 40, detail: 'socket' },
                { permission: 'spawn_process', file: 'helper.py', line: 3, detail: 'subprocess.run' },
            ],
        });
        render(<ModuleAuditNotice moduleName="game" />);
        expect(await screen.findByText(/uses capabilities it doesn't declare/)).toBeTruthy();
        expect(screen.getByText(/network access, running other programs/)).toBeTruthy();
        fireEvent.click(screen.getByRole('button', { name: 'Details (3)' }));
        expect(screen.getByText('helper.py:3')).toBeTruthy();
    });

    it('익스텐션도 같은 모양으로 경고하고, 실패한 검사는 다시 묻는다', async () => {
        window.api.extensionAudit = vi
            .fn()
            .mockResolvedValueOnce({ success: false, error: 'boom' })
            .mockResolvedValue({ undeclared: [{ permission: 'dynamic_code', file: 'main.py', line: 7 }] });
        const { container, unmount } = render(<ExtensionAuditNotice extId="music" name="Music" />);
        await waitFor(() => expect(window.api.extensionAudit).toHaveBeenCalledTimes(1));
        expect(container.innerHTML).toBe('');
        unmount();
        render(<ExtensionAuditNotice extId="music" name="Music" />);
        expect(await screen.findByText("Extension 'Music' uses capabilities it doesn't declare.")).toBeTruthy();
        expect(screen.getByText(/running dynamic code/)).toBeTruthy();
        expect(window.api.extensionAudit).toHaveBeenCalledWith('music');
    });
});
