import { act, fireEvent, render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import SettingsModal from '../components/Modals/SettingsModal';

describe('정보 페이지의 앱 버전', () => {
    it('앱 버전을 아직 모르면 지어낸 버전(0.1.0) 대신 비워 둔다', async () => {
        window.api.getAppVersion = vi.fn(() => new Promise(() => {}));
        window.api.getComponentInfo = vi.fn(() => new Promise(() => {}));
        const { container } = render(<SettingsModal isOpen onClose={vi.fn()} />);
        await act(async () => fireEvent.click(container.querySelector('.settings-about-btn')));
        expect(container.textContent).not.toContain('0.1.0');
        expect(container.querySelector('.about-version-badge')).toBeNull();
    });
});
