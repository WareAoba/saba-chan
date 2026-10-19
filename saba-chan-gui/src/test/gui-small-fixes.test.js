import { act, render, renderHook } from '@testing-library/react';
import { useEffect } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ExtensionSlot from '../components/ExtensionSlot';
import { useModalClose } from '../hooks/useModalClose';
import { useServerActions } from '../hooks/useServerActions';
import { getCachedRules, highlightLine } from '../utils/syntaxHighlight';

let slots = {};
vi.mock('../contexts/ExtensionContext', () => ({ useExtensions: () => ({ slots }) }));

describe('대체 강조(hl 그룹) 위치', () => {
    it('hl 글자가 매치 앞부분에도 있으면 hl 그룹 자리를 칠한다', () => {
        const rules = getCachedRules('t-hl', [{ name: 'r', pattern: 'x(?P<hl>x)', color: '#ff0000' }]);
        const segments = highlightLine('xx', rules);
        expect(segments.map((s) => [s.text, Boolean(s.style || s.token)])).toEqual([
            ['x', false],
            ['x', true],
        ]);
    });
});

describe('확장 슬롯', () => {
    it('앞의 컴포넌트가 빠져도 남은 컴포넌트를 새로 만들지 않는다', () => {
        const mounts = [];
        const A = () => null;
        const B = () => {
            useEffect(() => {
                mounts.push('B');
            }, []);
            return null;
        };
        slots = { s: [A, B] };
        const { rerender } = render(<ExtensionSlot slotId="s" />);
        slots = { s: [B] };
        rerender(<ExtensionSlot slotId="s" />);
        expect(mounts).toEqual(['B']);
    });
});

describe('모달 닫기 타이머', () => {
    it('닫히는 중에 모달이 사라지면 onClose를 뒤늦게 부르지 않는다', () => {
        vi.useFakeTimers();
        const onClose = vi.fn();
        const { result, unmount } = renderHook(() => useModalClose(onClose));
        act(() => result.current.requestClose());
        unmount();
        vi.advanceTimersByTime(500);
        expect(onClose).not.toHaveBeenCalled();
        vi.useRealTimers();
    });
});

describe('서버 만들기 (이름, 모듈) 호출', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = {
            serverList: vi.fn().mockResolvedValue({ servers: [] }),
            instanceCreate: vi.fn().mockResolvedValue({ success: true, id: 'b' }),
        };
    });

    it('두 번째 인자의 모듈로 만든다', async () => {
        const { result } = renderHook(() =>
            useServerActions({
                servers: [],
                setServers: vi.fn(),
                modules: [{ name: 'game' }],
                loading: false,
                setLoading: vi.fn(),
                setModal: vi.fn(),
                setProgressBar: vi.fn(),
                openConsole: vi.fn(),
                closeConsole: vi.fn(),
                setShowModuleManager: vi.fn(),
                formatUptime: vi.fn(),
            }),
        );
        await act(async () => result.current.handleAddServer('B', 'game'));
        expect(window.api.instanceCreate).toHaveBeenCalledWith(expect.objectContaining({ module_name: 'game' }));
    });
});
