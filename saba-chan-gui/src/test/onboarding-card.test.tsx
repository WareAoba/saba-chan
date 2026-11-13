import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useGuideStore } from '../components/guides/guides';
import { OnboardingCard, StartCoach } from '../components/servers/OnboardingCard';
import { useSettingsStore } from '../stores/useSettingsStore';
import { useUIStore } from '../stores/useUIStore';

const steps = (done: string[]) => ['discord', 'module', 'server', 'start'].map((id) => ({ id, done: done.includes(id), optional: false })).concat({ id: 'chatbot', done: false, optional: true });

const load = async (done: string[], next: string | null) => {
    window.api.onboarding = vi.fn().mockResolvedValue({ steps: steps(done), next });
    await act(() => useGuideStore.getState().refreshOnboarding());
};

describe('처음 시작하기 체크리스트 (GET /api/onboarding)', () => {
    beforeEach(() => {
        window.api = {} as unknown as typeof window.api;
        useUIStore.getState()._resetForTest();
        useSettingsStore.setState({ closedGuides: [] });
        useGuideStore.setState({ onboarding: null, step: null, coachDismissed: false });
    });

    it('남은 필수 단계가 없거나 닫았으면 그리지 않는다', async () => {
        await load(['discord', 'module', 'server', 'start'], null);
        const r = render(<OnboardingCard />);
        expect(r.container).toBeEmptyDOMElement();
        await load([], 'discord');
        act(() => useSettingsStore.setState({ closedGuides: ['checklist'] }));
        expect(r.container).toBeEmptyDOMElement();
    });

    it('디스코드 연결부터 순서대로 보이고, 다음 단계 버튼은 그 화면으로 보낸다', async () => {
        await load([], 'discord');
        render(<OnboardingCard />);
        expect(screen.getByText('0 / 4')).toBeInTheDocument();
        const names = screen.getAllByRole('listitem').map((li) => li.textContent);
        expect(names[0]).toMatch(/Connect Discord/);
        expect(names[3]).toMatch(/Start your server/);
        fireEvent.click(screen.getByRole('button', { name: 'Connect Discord' }));
        expect(useUIStore.getState().activeTab).toBe('discord');
    });

    it('게임 받기 단계는 모듈 받기 가이드를 연다', async () => {
        await load(['discord'], 'module');
        render(<OnboardingCard />);
        fireEvent.click(screen.getByRole('button', { name: 'Get game modules' }));
        expect(useGuideStore.getState().step).toBe('modules');
    });

    it('✕로 닫으면 설정에 남는다', async () => {
        await load([], 'discord');
        render(<OnboardingCard />);
        fireEvent.click(screen.getByRole('button', { name: 'Close' }));
        expect(useSettingsStore.getState().closedGuides).toContain('checklist');
    });

    it('다음 단계가 첫 시작이면 시작 버튼 말풍선을 보이고, 닫으면 숨긴다', async () => {
        await load(['discord', 'module', 'server'], 'start');
        render(<StartCoach />);
        expect(screen.getByText('Press Start to start the server')).toBeInTheDocument();
        fireEvent.click(screen.getByRole('button', { name: 'Close' }));
        expect(screen.queryByText('Press Start to start the server')).toBeNull();
    });
});
