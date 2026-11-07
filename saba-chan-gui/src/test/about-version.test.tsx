import { act, fireEvent, render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import SettingsModalImpl from '../components/Modals/SettingsModal';
import type { ComponentProps, ComponentType } from 'react';

/** 시험에 필요한 props만 넘기려고 느슨하게 받는다 */
const SettingsModal = SettingsModalImpl as ComponentType<Partial<ComponentProps<typeof SettingsModalImpl>>>;

describe('정보 페이지의 앱 버전', () => {
    it('앱 버전을 아직 모르면 지어낸 버전(0.1.0) 대신 비워 둔다', async () => {
        window.api.getAppVersion = vi.fn(() => new Promise(() => {})) as never;
        window.api.getComponentInfo = vi.fn(() => new Promise(() => {})) as never;
        const { container } = render(<SettingsModal isOpen onClose={vi.fn()} />);
        await act(async () => fireEvent.click(container.querySelector('#settings-tab-about') as HTMLElement));
        expect(container.textContent).not.toContain('0.1.0');
        expect(container.querySelector('.about-version-badge')).toBeNull();
    });
});
