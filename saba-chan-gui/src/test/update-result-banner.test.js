import { fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { UpdateResultBanner } from '../components/shell/UpdateResultBanner';

describe('UpdateResultBanner', () => {
    afterEach(() => {
        delete window.api.updaterLastResult;
    });

    it('데몬이 준비되기 전에는 묻지 않는다', () => {
        window.api.updaterLastResult = vi.fn().mockResolvedValue(null);
        render(<UpdateResultBanner daemonReady={false} />);
        expect(window.api.updaterLastResult).not.toHaveBeenCalled();
    });

    it('결과가 없으면 아무것도 그리지 않는다', async () => {
        window.api.updaterLastResult = vi.fn().mockResolvedValue(null);
        render(<UpdateResultBanner daemonReady />);
        await waitFor(() => expect(window.api.updaterLastResult).toHaveBeenCalledWith({ clear: false }));
        expect(document.querySelector('.update-banner')).toBeNull();
    });

    it('실패 결과를 보여주고, 닫으면 clear 한다', async () => {
        // 데몬이 실제로 돌려주는 봉투 모양 그대로
        window.api.updaterLastResult = vi.fn().mockResolvedValue({
            ok: true,
            result: {
                timestamp: 1,
                success: false,
                updated_components: ['saba-chan-gui'],
                failed: ['saba-core'],
                message: 'busy',
            },
        });
        render(<UpdateResultBanner daemonReady />);
        await waitFor(() => expect(document.querySelector('.update-banner.tone-stop')).not.toBeNull());
        expect(document.querySelector('.update-banner').textContent).toContain('saba-core');
        fireEvent.click(document.querySelector('.update-banner .icon-btn'));
        expect(window.api.updaterLastResult).toHaveBeenLastCalledWith({ clear: true });
        expect(document.querySelector('.update-banner')).toBeNull();
    });
});
