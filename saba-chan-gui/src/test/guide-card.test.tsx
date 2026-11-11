import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { GuideCard } from '../components/guides/GuideCard';
import { type GuideItem, pendingGuides, startGuide, useGuideStore } from '../components/guides/guides';
import { Spotlight } from '../components/guides/Spotlight';
import { useUIStore } from '../stores/useUIStore';

const guide = (id: string, over: Partial<GuideItem> = {}): GuideItem => ({
    id,
    group: 'care',
    icon: 'save',
    title: `T-${id}`,
    summary: `S-${id}`,
    steps: [{ text: `${id} one` }, { text: `${id} two` }],
    done: false,
    new: false,
    step: 0,
    ...over,
});

const list = () => [
    guide('a', { group: 'start' }),
    guide('fin', { group: 'start', done: true }),
    guide('b'),
    guide('fresh', { new: true }),
    guide('going', { step: 1 }),
];

describe('가이드 카드 (사용자 11-11)', () => {
    beforeEach(() => {
        window.api = { guideUpdate: vi.fn().mockResolvedValue({ ok: true }), guides: vi.fn() } as unknown as typeof window.api;
        useUIStore.getState()._resetForTest();
        useGuideStore.setState({ guides: list(), active: null });
    });

    it('안 끝낸 것만: 진행 중 → new! → 나머지 (같은 차례면 데몬 순서)', () => {
        expect(pendingGuides(list()).map((g) => g.id)).toEqual(['going', 'fresh', 'a', 'b']);
    });

    it('평소에는 "가이드" 한 줄, 누르면 세 개까지 펼치고 new!는 글자로만', () => {
        render(<GuideCard />);
        expect(screen.queryByText('T-going')).toBeNull();
        fireEvent.click(screen.getByRole('button', { name: /Guides/ }));
        const titles = screen.getAllByRole('button').map((b) => b.textContent);
        expect(titles.filter((x) => x?.startsWith('T-'))).toEqual(['T-goingStep 2 of 2', 'T-freshnew!S-fresh', 'T-aS-a']);
        expect(screen.getByText('new!').tagName).toBe('SPAN');
    });

    it('펼칠 때와 새 가이드를 받았을 때 목록을 다시 묻는다 (완료·new!가 늦게 보이지 않게)', async () => {
        let push: (raw: unknown) => void = () => {};
        window.api.onDaemonEvent = vi.fn((cb: (raw: unknown) => void) => {
            push = cb;
            return () => {};
        }) as never;
        window.api.guides = vi.fn().mockResolvedValue({ guides: list() }) as never;
        render(<GuideCard />);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Guides/ })));
        expect(window.api.guides).toHaveBeenCalledTimes(1);
        await act(async () => push({ type: 'guides.updated', data: { count: 1 } }));
        expect(window.api.guides).toHaveBeenCalledTimes(2);
    });

    it('더 보기: 끝낸 것까지 묶음별 전체 목록, 고르면 그 가이드를 연다', () => {
        render(<GuideCard />);
        fireEvent.click(screen.getByRole('button', { name: /Guides/ }));
        fireEvent.click(screen.getByRole('button', { name: 'More' }));
        const sheet = screen.getByRole('dialog');
        expect(within(sheet).getAllByRole('heading').map((h) => h.textContent)).toEqual(['Getting started', 'Server care']);
        expect(within(sheet).getByText('T-fin')).toBeInTheDocument();
        fireEvent.click(within(sheet).getByText('T-fresh'));
        expect(useGuideStore.getState().active).toEqual({ id: 'fresh', step: 0 });
        // 한 번 열면 new!가 꺼진다 (데몬에도 알린다)
        expect(window.api.guideUpdate).toHaveBeenCalledWith('fresh', { opened: true });
        expect(useGuideStore.getState().guides?.find((g) => g.id === 'fresh')?.new).toBe(false);
    });
});

describe('화면 강조', () => {
    beforeEach(() => {
        window.api = {
            guideUpdate: vi.fn().mockResolvedValue({ ok: true }),
            guides: vi.fn().mockResolvedValue({ guides: [] }),
        } as unknown as typeof window.api;
        useUIStore.getState()._resetForTest();
        useGuideStore.setState({
            guides: [guide('b', { steps: [{ spot: 'here', text: 'look here' }, { text: 'centered' }] })],
            active: null,
        });
    });

    it('짚을 자리를 밝히고 말풍선에 설명, 다음 → 마지막은 다 봤어요', async () => {
        const target = document.createElement('button');
        target.dataset.spot = 'here';
        document.body.appendChild(target);
        render(<Spotlight />);
        act(() => startGuide('b'));
        await act(async () => {});
        expect(screen.getByText('look here')).toBeInTheDocument();
        expect(screen.getByText('1 / 2')).toBeInTheDocument();
        expect(document.querySelector('.spotlight-hole')).not.toBeNull();
        fireEvent.click(screen.getByRole('button', { name: 'Next' }));
        await act(async () => {});
        expect(screen.getByText('centered')).toBeInTheDocument();
        expect(window.api.guideUpdate).toHaveBeenCalledWith('b', { step: 1 });
        fireEvent.click(screen.getByRole('button', { name: 'Got it' }));
        expect(useGuideStore.getState().active).toBeNull();
        expect(window.api.guideUpdate).toHaveBeenCalledWith('b', { finished: true, step: 0 });
        target.remove();
    });

    it('말풍선 밖을 누르면 바로 닫히고, 보던 단계는 남는다', async () => {
        render(<Spotlight />);
        act(() => {
            useGuideStore.setState((s) => ({ guides: s.guides?.map((g) => ({ ...g, step: 1 })) ?? null }));
            startGuide('b');
        });
        await act(async () => {});
        expect(screen.getByText('centered')).toBeInTheDocument();
        fireEvent.click(screen.getAllByTestId('spotlight-outside')[0]);
        expect(useGuideStore.getState().active).toBeNull();
        expect(screen.queryByTestId('spotlight')).toBeNull();
        expect(useGuideStore.getState().guides?.[0].step).toBe(1);
    });

    it('화면을 넘겨받는 단계는 말풍선을 먼저 보이고, 열기를 눌러야 그 화면으로 넘기며 끝난다', async () => {
        const open = vi.fn();
        window.addEventListener('saba:open-sonar', open);
        useGuideStore.setState({ guides: [guide('s', { steps: [{ action: 'sonar', text: 'press ctrl k' }] })], active: null });
        render(<Spotlight />);
        act(() => startGuide('s'));
        await act(async () => {});
        expect(screen.getByText('press ctrl k')).toBeInTheDocument();
        expect(open).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole('button', { name: 'Open' }));
        expect(open).toHaveBeenCalledTimes(1);
        expect(useGuideStore.getState().active).toBeNull();
        window.removeEventListener('saba:open-sonar', open);
    });

    it('말풍선이 뜨면 주 버튼에 포커스가 간다 (가려진 화면을 Tab으로 지나지 않게)', async () => {
        const target = document.createElement('button');
        target.dataset.spot = 'here';
        document.body.appendChild(target);
        target.focus();
        render(<Spotlight />);
        act(() => startGuide('b'));
        await act(async () => {});
        expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Next' }));
        target.remove();
    });

    it('받은 가이드의 action·icon이 객체 기본 속성 이름이어도 그 일로 여기지 않는다', async () => {
        useGuideStore.setState({
            guides: [guide('p', { icon: 'constructor', steps: [{ action: 'toString', text: 'proto' }] })],
            active: null,
        });
        render(<Spotlight />);
        act(() => startGuide('p'));
        await act(async () => {});
        expect(screen.getByText('proto')).toBeInTheDocument();
        fireEvent.click(screen.getByRole('button', { name: 'Got it' }));
        expect(useGuideStore.getState().active).toBeNull();
    });

    it('Esc로도 닫힌다', async () => {
        render(<Spotlight />);
        act(() => startGuide('b'));
        await act(async () => {});
        fireEvent.keyDown(window, { key: 'Escape' });
        expect(useGuideStore.getState().active).toBeNull();
    });
});
