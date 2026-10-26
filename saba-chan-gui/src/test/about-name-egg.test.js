import { act, fireEvent, render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import SettingsModal from '../components/Modals/SettingsModal';

describe('정보 페이지 로고 이스터에그', () => {
    it('로고를 다섯 번 누르면 이름 해명이 뜨고, 네 번까지는 뜨지 않는다', async () => {
        window.api.getAppVersion = vi.fn(() => new Promise(() => {}));
        window.api.getComponentInfo = vi.fn(() => new Promise(() => {}));
        window.showToast = vi.fn();
        const { container } = render(<SettingsModal isOpen onClose={vi.fn()} />);
        await act(async () => fireEvent.click(container.querySelector('.settings-about-btn')));
        const logo = container.querySelector('.about-app-logo');
        for (let i = 0; i < 4; i++) fireEvent.click(logo);
        expect(window.showToast).not.toHaveBeenCalled();
        fireEvent.click(logo);
        expect(window.showToast).toHaveBeenCalledTimes(1);
        expect(window.showToast.mock.calls[0][0]).toContain('VTuber');
    });
});
