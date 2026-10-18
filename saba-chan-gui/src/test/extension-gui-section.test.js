import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ExtensionGuiSection } from '../components/Modals/ExtensionGuiSection';
import ExtensionContext from '../contexts/ExtensionContext';

function mount(ctx) {
    return render(
        <ExtensionContext.Provider value={ctx}>
            <ExtensionGuiSection />
        </ExtensionContext.Provider>,
    );
}

describe('설정 > 익스텐션 화면 허락', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api.sandboxListExtensionGui = vi.fn().mockResolvedValue({
            components: [{ component: 'ext-music', fingerprint: 'f', current: true }],
        });
        window.api.sandboxRevokeExtensionGui = vi.fn().mockResolvedValue({ success: true, component: 'ext-music' });
    });
    afterEach(() => {
        delete window.api.sandboxListExtensionGui;
        delete window.api.sandboxRevokeExtensionGui;
    });

    it('허락한 목록은 철회할 수 있고, 기다리는 익스텐션은 여기서 허락할 수 있다', async () => {
        const approveExtensionGui = vi.fn().mockResolvedValue(true);
        mount({ guiPending: [{ id: 'docker' }], approveExtensionGui });
        await waitFor(() => expect(screen.getByText('ext-music')).toBeTruthy());

        await act(async () => fireEvent.click(screen.getByRole('button', { name: /docker/ })));
        expect(approveExtensionGui).toHaveBeenCalledWith('docker');

        await act(async () => fireEvent.click(screen.getByRole('button', { name: /ext-music/ })));
        expect(window.api.sandboxRevokeExtensionGui).toHaveBeenCalledWith('ext-music');
    });
});
