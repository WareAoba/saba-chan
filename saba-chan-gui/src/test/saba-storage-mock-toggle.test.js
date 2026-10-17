/**
 * SabaStorage 컴포넌트의 MOCK/REAL 토글이 새 Relay 기반 UpdateConfig 필드
 * (`relay_url`)만 전송하는지 검증한다.
 *
 * Stage A 리팩터로 `api_base_url`, `github_owner`, `github_repo` 는
 * UpdateConfig 에서 제거되었다. 토글 핸들러가 더 이상 이런 필드를 보내지
 * 않아야 하며, 대신 `relay_url` 을 사용해야 한다.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Stub for ExtensionContext (SabaStorage imports useExtensions)
vi.mock('../contexts/ExtensionContext', () => ({
    useExtensions: () => ({
        extensions: [],
        loading: false,
        refresh: vi.fn(),
        installExtension: vi.fn(),
        toggleExtension: vi.fn(),
    }),
}));

import SabaStorage from '../components/Modals/SabaStorage';

const setConfigSpy = vi.fn().mockResolvedValue({ ok: true });
const getConfigSpy = vi.fn().mockResolvedValue({
    config: {
        enabled: true,
        relay_url: 'https://saba-chan.online',
        channel: 'stable',
        platform: 'windows-x64',
        client_id: '00000000',
    },
});

beforeEach(() => {
    setConfigSpy.mockClear();
    getConfigSpy.mockClear();
    window.api = new Proxy(
        {
            updaterSetConfig: setConfigSpy,
            updaterGetConfig: getConfigSpy,
            updaterStatus: vi.fn().mockResolvedValue({ ok: true, components: [] }),
            updaterCheck: vi.fn().mockResolvedValue({ ok: true, components: [] }),
            updaterDownloadProgress: vi.fn().mockResolvedValue({ ok: true, active: false }),
        },
        {
            get(target, prop) {
                if (typeof prop === 'symbol') return target[prop];
                if (!(prop in target)) target[prop] = vi.fn().mockResolvedValue({});
                return target[prop];
            },
        },
    );
});

afterEach(() => {
    delete window.api;
});

describe('SabaStorage MOCK/REAL toggle (Relay refactor)', () => {
    it('초기 로드 시 isMock 판정에 relay_url 을 사용한다', async () => {
        getConfigSpy.mockResolvedValueOnce({
            config: {
                enabled: true,
                relay_url: 'http://127.0.0.1:9876',
                channel: 'stable',
                platform: 'windows-x64',
                client_id: '00000000',
            },
        });

        render(<SabaStorage onBack={() => {}} isExiting={false} devMode />);

        await waitFor(() => {
            // MOCK 라벨이 표시되어야 함 (isMock=true)
            expect(screen.getByText('MOCK')).toBeTruthy();
        });
    });

    it('MOCK 으로 토글하면 relay_url 만 전송하고 legacy 필드는 보내지 않는다', async () => {
        render(<SabaStorage onBack={() => {}} isExiting={false} devMode />);

        // 초기 cfg 로드 완료 대기 (REAL 상태)
        await waitFor(() => expect(screen.getByText('REAL')).toBeTruthy());

        const mockToggleLabel = document.querySelector('.update-mock-toggle');
        const toggle = mockToggleLabel.querySelector('input[type="checkbox"]');
        await act(async () => {
            fireEvent.click(toggle);
        });

        await waitFor(() => expect(setConfigSpy).toHaveBeenCalled());

        const lastCall = setConfigSpy.mock.calls.find((c) => c[0] && Object.hasOwn(c[0], 'relay_url'));
        expect(lastCall, 'relay_url 을 포함한 호출이 있어야 함').toBeTruthy();
        const payload = lastCall[0];

        expect(payload.relay_url).toMatch(/^http:\/\/127\.0\.0\.1/);
        expect(payload).not.toHaveProperty('api_base_url');
        expect(payload).not.toHaveProperty('github_owner');
        expect(payload).not.toHaveProperty('github_repo');
    });

    it('REAL 로 토글하면 relay_url 을 빈 문자열(=기본값 사용)로 리셋한다', async () => {
        getConfigSpy.mockResolvedValueOnce({
            config: {
                enabled: true,
                relay_url: 'http://127.0.0.1:9876',
                channel: 'stable',
                platform: 'windows-x64',
                client_id: '00000000',
            },
        });

        render(<SabaStorage onBack={() => {}} isExiting={false} devMode />);

        // 초기 MOCK 상태 로드 대기
        await waitFor(() => expect(screen.getByText('MOCK')).toBeTruthy());

        const mockToggleLabel = document.querySelector('.update-mock-toggle');
        const toggle = mockToggleLabel.querySelector('input[type="checkbox"]');
        await act(async () => {
            fireEvent.click(toggle);
        });

        await waitFor(() => expect(setConfigSpy).toHaveBeenCalled());

        const lastCall = setConfigSpy.mock.calls.find((c) => c[0] && Object.hasOwn(c[0], 'relay_url'));
        expect(lastCall, 'relay_url 을 포함한 호출이 있어야 함').toBeTruthy();
        const payload = lastCall[0];

        expect(payload.relay_url).toBe('');
        expect(payload).not.toHaveProperty('api_base_url');
        expect(payload).not.toHaveProperty('github_owner');
        expect(payload).not.toHaveProperty('github_repo');
    });
});
