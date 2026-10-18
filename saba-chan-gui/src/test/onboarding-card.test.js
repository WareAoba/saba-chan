import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { OnboardingCard } from '../components/servers/OnboardingCard';
import { useUIStore } from '../stores/useUIStore';

const steps = (done) => [
    { id: 'module', done: done.includes('module'), optional: false },
    { id: 'server', done: done.includes('server'), optional: false },
    { id: 'discord', done: done.includes('discord'), optional: false },
    { id: 'chatbot', done: false, optional: true },
];

describe('다음 할 일 안내 (GET /api/onboarding)', () => {
    beforeEach(() => {
        window.api = {};
    });

    it('채널이 없거나 남은 필수 단계가 없으면 그리지 않는다', async () => {
        const { container } = render(<OnboardingCard onAddServer={vi.fn()} />);
        expect(container).toBeEmptyDOMElement();
        window.api.onboarding = vi
            .fn()
            .mockResolvedValue({ steps: steps(['module', 'server', 'discord']), next: null });
        const r = render(<OnboardingCard onAddServer={vi.fn()} />);
        await act(async () => {});
        expect(r.container).toBeEmptyDOMElement();
    });

    it('다음 단계를 보이고, 버튼은 그 화면으로 보낸다', async () => {
        window.api.onboarding = vi.fn().mockResolvedValue({ steps: steps(['module', 'server']), next: 'discord' });
        render(<OnboardingCard onAddServer={vi.fn()} />);
        await act(async () => {});
        expect(screen.getByText('2/3')).toBeInTheDocument();
        fireEvent.click(screen.getByRole('button', { name: /Connect Discord/i }));
        expect(useUIStore.getState().activeTab).toBe('discord');
    });

    it('모듈·서버 단계는 서버 추가 화면을 연다', async () => {
        window.api.onboarding = vi.fn().mockResolvedValue({ steps: steps([]), next: 'module' });
        const onAddServer = vi.fn();
        render(<OnboardingCard onAddServer={onAddServer} />);
        await act(async () => {});
        fireEvent.click(screen.getByRole('button', { name: /Add a game/i }));
        expect(onAddServer).toHaveBeenCalled();
    });
});
