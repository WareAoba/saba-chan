/**
 * 구성요소 탭은 "업데이트" 단추 하나로 모두 올린다 (사용자 11-14).
 * 받기·적용·카드별 단추를 따로 두면 무엇이 다운로드고 무엇이 업데이트인지 알기 어렵다.
 * 모듈·익스텐션 업데이트도 같은 요청에 들어간다 (UP1: 0.1.8→0.1.9에서 앱만 바뀌었다).
 */
import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../contexts/ExtensionContext', () => ({
    useExtensions: () => ({ extensions: [], loading: false, refresh: vi.fn() }),
}));

import SabaStorage from '../components/Modals/SabaStorage';

const components = [
    { component: 'saba-core', kind: 'app', display_name: 'Core', current_version: '0.1.13', latest_version: '0.1.14', update_available: true },
    { component: 'cli', kind: 'app', display_name: 'CLI', current_version: '0.1.13', latest_version: '0.1.14', update_available: true },
    { component: 'gui', kind: 'app', display_name: 'GUI', current_version: '0.1.14', update_available: false },
    { component: 'module-minecraft', kind: 'module', display_name: 'Minecraft', update_available: true },
];

beforeEach(() => {
    window.showToast = vi.fn();
    window.api = new Proxy<Record<string | symbol, unknown>>(
        {
            updaterStatus: vi.fn().mockResolvedValue({ success: true, components }),
            updaterDownload: vi.fn().mockResolvedValue({ success: true }),
            updaterApply: vi.fn().mockResolvedValue({ success: true, applied: ['saba-core', 'cli', 'module-minecraft'], needs_updater: [], requires_updater: false }),
        },
        {
            get(target, prop) {
                if (typeof prop === 'symbol') return target[prop];
                if (!(prop in target)) target[prop] = vi.fn().mockResolvedValue({});
                return target[prop];
            },
        },
    ) as unknown as typeof window.api;
});

afterEach(() => {
    delete (window as { api?: unknown }).api;
});

describe('업데이트 단추 하나', () => {
    it('카드마다 단추가 없고, 단추 하나가 전부(모듈·익스텐션 포함)를 받아 적용한다', async () => {
        render(<SabaStorage />);
        await waitFor(() => expect(document.querySelector('.ss-update-btn')).not.toBeNull());
        const tab = document.querySelector('.ss-hero')!.parentElement!;
        expect(tab.querySelectorAll('.ss-card')).toHaveLength(3);
        expect(tab.querySelectorAll('.ss-card button')).toHaveLength(0);
        expect(document.querySelector('.ss-hero')!.textContent).toContain('0.1.14');

        await act(async () => fireEvent.click(document.querySelector('.ss-update-btn')!));
        // 코어가 바뀌면 다시 시작한다고 먼저 묻는다
        const confirm = await waitFor(() => {
            const b = document.querySelector<HTMLButtonElement>('.saba-confirm .btn-primary');
            expect(b).not.toBeNull();
            return b!;
        });
        await act(async () => fireEvent.click(confirm));

        await waitFor(() => expect(window.api.updaterApply).toHaveBeenCalled());
        // 빈 목록 = 데몬이 아는 업데이트 전부
        expect(window.api.updaterDownload).toHaveBeenCalledWith([]);
        expect(window.api.updaterApply).toHaveBeenCalledWith([]);
    });

    it('모두 최신이면 업데이트 단추 없이 최신 버전을 보인다', async () => {
        vi.mocked(window.api.updaterStatus).mockResolvedValue({
            success: true,
            components: components.map((c) => ({ ...c, update_available: false, current_version: '0.1.14' })),
        });
        render(<SabaStorage />);
        await waitFor(() => expect(document.querySelector('.ss-hero .ss-ok-chip')).not.toBeNull());
        expect(document.querySelector('.ss-update-btn')).toBeNull();
        expect(document.querySelector('.ss-hero')!.textContent).toContain('v0.1.14');
    });
});
