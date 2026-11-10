/**
 * 상단 바 알약 공용 움직임 (시안 v8): 원이 튕기며 생긴 뒤 왼쪽으로 늘어나고, 사라질 때는 원으로 줄었다가 없어진다.
 * 작업 표시·Python 준비·업데이트 알약이 모두 같은 AppBarPill을 쓴다. 움직임 줄이기면 움직이지 않는다
 */
import { act, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AppBarPill } from '../components/shell/AppBarPill';
import { RuntimePreflight } from '../components/shell/RuntimePreflight';
import { TaskIndicator } from '../components/shell/TaskIndicator';
import { useUIStore } from '../stores/useUIStore';

type Finish = () => void;
let finishers: Finish[] = [];
const animate = vi.fn();
const realMatchMedia = window.matchMedia;

function setReducedMotion(reduce: boolean) {
    window.matchMedia = ((query: string) => ({
        ...realMatchMedia(query),
        matches: query === '(prefers-reduced-motion: reduce)' ? reduce : realMatchMedia(query).matches,
    })) as typeof window.matchMedia;
}

beforeEach(() => {
    finishers = [];
    animate.mockReset().mockImplementation(() => {
        let done!: Finish;
        const finished = new Promise<void>((r) => {
            done = r;
        });
        finishers.push(done);
        return { finished, cancel: vi.fn() };
    });
    (HTMLElement.prototype as unknown as { animate: unknown }).animate = animate;
    (HTMLElement.prototype as unknown as { getAnimations: unknown }).getAnimations = () => [];
    setReducedMotion(false);
});

afterEach(() => {
    delete (HTMLElement.prototype as unknown as { animate?: unknown }).animate;
    delete (HTMLElement.prototype as unknown as { getAnimations?: unknown }).getAnimations;
    window.matchMedia = realMatchMedia;
    vi.useRealTimers();
});

const pill = (visible: boolean, label = '업데이트 적용 중...') => (
    <AppBarPill visible={visible} tone="applying" icon={<span className="ic-test" />} label={label} />
);

const finishAll = async () => {
    await act(async () => {
        for (const f of finishers.splice(0)) f();
    });
};

describe('알약 공용 움직임', () => {
    it('나타날 때 원이 튕기며 생긴다 (크기 0 → 1)', () => {
        render(pill(true));
        const frames = animate.mock.calls.map(([f]) => f);
        expect(frames).toContainEqual([{ transform: 'scale(0)' }, { transform: 'scale(1)' }]);
    });

    it('원이 생긴 뒤 30px 원에서 제 너비로 늘어난다', () => {
        vi.useFakeTimers();
        render(pill(true));
        act(() => vi.advanceTimersByTime(300));
        const widths = animate.mock.calls.map(([f]) => f).filter((f) => 'width' in f[0]);
        expect(widths[0][0]).toEqual({ width: '30px' });
    });

    it('사라질 때는 움직임이 끝날 때까지 남아 있다가 없어진다', async () => {
        const { rerender, container } = render(pill(true));
        await finishAll();
        rerender(pill(false));
        expect(container.querySelector('.appbar-pill')).not.toBeNull();
        // 원으로 줄어들기 → 작아지며 사라지기
        await finishAll();
        await finishAll();
        expect(container.querySelector('.appbar-pill')).toBeNull();
    });

    it('사라지는 중에 다시 보이면 남는다', async () => {
        const { rerender, container } = render(pill(true));
        await finishAll();
        rerender(pill(false));
        rerender(pill(true));
        await finishAll();
        await finishAll();
        expect(container.querySelector('.appbar-pill')).not.toBeNull();
    });

    it('움직임 줄이기면 움직이지 않고 바로 나타나고 사라진다', () => {
        setReducedMotion(true);
        const { rerender, container } = render(pill(true));
        expect(container.querySelector('.appbar-pill')).not.toBeNull();
        rerender(pill(false));
        expect(container.querySelector('.appbar-pill')).toBeNull();
        expect(animate).not.toHaveBeenCalled();
    });
});

describe('공용 움직임을 쓰는 알약', () => {
    afterEach(() => act(() => useUIStore.getState().clearProgressBar()));

    it('작업 표시는 공용 알약이다', () => {
        const { container } = render(<TaskIndicator />);
        act(() => useUIStore.getState().setProgressBar({ message: '시작 중', indeterminate: true }));
        expect(container.querySelector('.appbar-pill.appbar-task')).not.toBeNull();
    });

    it('Python 준비 표시는 공용 알약이다', async () => {
        window.api.runtimePreflight = vi.fn().mockResolvedValue({ phase: 'preflight-python', percent: 40 });
        const { container, findByText } = render(<RuntimePreflight daemonStatus="running" />);
        await findByText('40%');
        expect(container.querySelector('.appbar-pill')).not.toBeNull();
        delete (window.api as unknown as Record<string, unknown>).runtimePreflight;
    });
});
