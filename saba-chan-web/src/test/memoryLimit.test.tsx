import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { SabaSettings } from '../components/SabaSettings';
import { initI18n } from '../i18n';
import { useApp } from '../store';
import { asServer, mockApi } from './mocks';

// 서버가 도는 PC의 OS가 메모리 상한을 걸 수 없으면 데몬이 저장하지 않고 memory_limit_unsupported 로 답한다
describe('웹 메모리 상한 저장', () => {
    beforeAll(() => initI18n('en'));
    beforeEach(() => useApp.setState({ toasts: [] }));

    it('데몬이 이 OS에서는 걸 수 없다고 하면 그 까닭을 알린다', async () => {
        const instanceUpdateSettings = vi.fn(async () => ({
            success: false,
            error: 'memory_limit_mb is only enforced on Windows',
            error_code: 'memory_limit_unsupported',
        }));
        render(<SabaSettings api={mockApi({ instanceUpdateSettings })} server={asServer({ id: 'a', name: 'S' })} running={false} />);
        fireEvent.change(screen.getByPlaceholderText('No limit'), { target: { value: '4096' } });
        await act(async () => {
            fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        });
        expect(instanceUpdateSettings).toHaveBeenCalledWith('a', { memory_limit_mb: '4096' });
        expect(useApp.getState().toasts.at(-1)).toMatchObject({ message: expect.stringMatching(/apply only on Windows/), tone: 'error' });
    });

    // 데몬이 이 서버에 상한을 걸 수 없다고 알리면(memory_limit_supported:false) 칸을 잠그고 이유를 알린다
    it('상한을 걸 수 없는 PC면 칸을 잠그고 이유를 한 줄로 알린다', () => {
        const server = asServer({ id: 'a', name: 'S', memory_limit_mb: null, memory_limit_supported: false });
        render(<SabaSettings api={mockApi({})} server={server} running={false} />);
        expect((screen.getByPlaceholderText('No limit') as HTMLInputElement).disabled).toBe(true);
        expect(screen.getByText(/memory limit cannot be used/)).toBeTruthy();
        expect(screen.queryByText(/Applies on Windows only/)).toBeNull();
    });

    it('걸리지 않는 예전 값이 남았으면 알리고, "비우기"는 빈 상한을 저장한다', async () => {
        const instanceUpdateSettings = vi.fn(async () => ({ success: true }));
        const server = asServer({ id: 'a', name: 'S', memory_limit_mb: 4096, memory_limit_supported: false });
        render(<SabaSettings api={mockApi({ instanceUpdateSettings })} server={server} running={false} />);
        expect(screen.getByText(/not applied now/)).toBeTruthy();
        await act(async () => {
            fireEvent.click(screen.getByRole('button', { name: 'Clear' }));
        });
        expect(instanceUpdateSettings).toHaveBeenCalledWith('a', { memory_limit_mb: null });
    });
});
