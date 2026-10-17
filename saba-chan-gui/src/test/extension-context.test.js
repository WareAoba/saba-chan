/**
 * ExtensionContext — 초기 로드 재시도 테스트
 *
 * 콜드부트 시 데몬 IPC가 준비되기 전에 extensionList가 에러를 반환하면
 * 익스텐션 GUI가 세션 내내 사라지던 버그의 재발을 방지한다.
 * (에러 응답은 빈 목록으로 위장하지 않고 재시도한다)
 */
import { render, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ExtensionProvider, useExtensions } from '../contexts/ExtensionContext';

function createApiMock(overrides = {}) {
    const base = { ...overrides };
    return new Proxy(base, {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (!(prop in target)) target[prop] = vi.fn().mockResolvedValue({});
            return target[prop];
        },
    });
}

let captured;
function Probe() {
    captured = useExtensions();
    return null;
}

beforeEach(() => {
    vi.clearAllMocks();
    captured = null;
    window.showToast = vi.fn();
});

describe('ExtensionContext 초기 로드', () => {
    it('에러 응답이면 성공할 때까지 재시도한다', async () => {
        const listMock = vi.fn()
            .mockResolvedValueOnce({ error: 'connect ECONNREFUSED' })
            .mockResolvedValue({ extensions: [{ id: 'docker', enabled: false, version: '1.0.0' }] });
        window.api = createApiMock({ extensionList: listMock });

        render(
            <ExtensionProvider>
                <Probe />
            </ExtensionProvider>,
        );

        // 1초 간격 재시도 → 두 번째 호출에서 성공
        await waitFor(() => expect(listMock).toHaveBeenCalledTimes(2), { timeout: 5000 });
        await waitFor(() => expect(captured.extensions).toHaveLength(1));
        expect(captured.extensions[0].id).toBe('docker');
    });

    it('정상 응답이면 즉시 목록이 반영된다', async () => {
        const listMock = vi.fn().mockResolvedValue({ extensions: [{ id: 'a', enabled: false, version: '1.0.0' }] });
        window.api = createApiMock({ extensionList: listMock });

        render(
            <ExtensionProvider>
                <Probe />
            </ExtensionProvider>,
        );

        await waitFor(() => expect(captured.extensions).toHaveLength(1));
        expect(listMock).toHaveBeenCalledTimes(1);
    });
});
