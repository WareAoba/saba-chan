import { act, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { AutostartSetting } from '../components/Modals/AutostartSetting';
import { pickOption } from './dropdown';

describe('설정 > 일반: 로그인할 때 자동 실행 (/api/system/autostart)', () => {
    beforeEach(() => {
        window.api = {
            systemAutostart: vi.fn().mockResolvedValue({ ok: true, supported: true, mode: 'off' }),
            systemAutostartSet: vi.fn(),
        };
    });

    it('지금 값을 보이고, 고르면 데몬에 바로 저장한다', async () => {
        window.api.systemAutostartSet.mockResolvedValue({
            ok: true,
            supported: true,
            mode: 'daemon',
        });
        await act(async () => render(<AutostartSetting />));
        const select = screen.getByRole('combobox', { name: 'Start at sign-in' });
        expect(select).toHaveTextContent('Off');
        pickOption(select, 'Daemon only');
        await act(async () => {});
        expect(window.api.systemAutostartSet).toHaveBeenCalledWith('daemon');
        expect(select).toHaveTextContent('Daemon only');
    });

    it('데몬이 거절하면 이유를 번역해 보이고 값은 그대로 둔다', async () => {
        window.api.systemAutostartSet.mockResolvedValue({
            ok: false,
            error_code: 'autostart_gui_missing',
            error: 'C:/x',
        });
        await act(async () => render(<AutostartSetting />));
        const select = screen.getByRole('combobox', { name: 'Start at sign-in' });
        pickOption(select, 'Everything with the GUI');
        await act(async () => {});
        expect(screen.getByRole('alert')).toHaveTextContent('The GUI is not installed');
        expect(select).toHaveTextContent('Off');
    });

    it('이 OS에서 못 하면 보이지 않는다', async () => {
        window.api.systemAutostart.mockResolvedValue({
            ok: true,
            supported: false,
            mode: 'off',
        });
        const { container } = await act(async () => render(<AutostartSetting />));
        expect(container).toBeEmptyDOMElement();
    });
});
