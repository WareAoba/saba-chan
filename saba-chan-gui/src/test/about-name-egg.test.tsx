import { act, fireEvent, render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import SettingsModalImpl from '../components/Modals/SettingsModal';
import type { ComponentProps, ComponentType } from 'react';

/** 시험에 필요한 props만 넘기려고 느슨하게 받는다 */
const SettingsModal = SettingsModalImpl as ComponentType<Partial<ComponentProps<typeof SettingsModalImpl>>>;

describe('정보 페이지 로고 이스터에그', () => {
    it('로고를 다섯 번 누르면 이름 해명이 뜨고, 네 번까지는 뜨지 않는다', async () => {
        window.api.getAppVersion = vi.fn(() => new Promise(() => {})) as never;
        window.api.getComponentInfo = vi.fn(() => new Promise(() => {})) as never;
        window.showToast = vi.fn();
        const { container } = render(<SettingsModal isOpen onClose={vi.fn()} />);
        await act(async () => fireEvent.click(container.querySelector('.settings-about-btn') as HTMLElement));
        const logo = container.querySelector('.about-app-logo') as HTMLElement;
        for (let i = 0; i < 4; i++) fireEvent.click(logo);
        expect(window.showToast).not.toHaveBeenCalled();
        fireEvent.click(logo);
        expect(window.showToast).toHaveBeenCalledTimes(1);
        expect(vi.mocked(window.showToast).mock.calls[0][0]).toContain('VTuber');
    });
});
