import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { clearModuleAuditCache, ModuleAuditNotice } from '../components/servers/ModuleAuditNotice';

describe('ModuleAuditNotice (데몬 M12)', () => {
    afterEach(() => {
        delete window.api.moduleAudit;
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
});
