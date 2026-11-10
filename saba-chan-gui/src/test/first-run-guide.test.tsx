import { act, fireEvent, render, screen } from '@testing-library/react';
import type { ComponentType } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useGuideStore, openGuide } from '../components/guides/guides';
import { ServersPage as ServersPageImpl } from '../components/servers/ServersPage';
import { useServerStore } from '../stores/useServerStore';
import { useSettingsStore } from '../stores/useSettingsStore';
import { useUIStore } from '../stores/useUIStore';

const ServersPage = ServersPageImpl as unknown as ComponentType<Record<string, unknown>>;
const page = (modules: { name: string }[] = []) => (
    <ServersPage servers={[]} modules={modules} onAddServer={vi.fn()} drag={{ cardRefs: { current: {} } }} />
);

// 사용자 11-08~10: 처음 켜면 디스코드 연결 → 쓰던 서버 불러오기 / 새로 만들기, 모듈이 없으면 모듈 받기를 먼저
describe('처음 시작하기 가이드', () => {
    beforeEach(() => {
        window.api = {
            onboarding: vi.fn().mockResolvedValue({ steps: [{ id: 'discord', done: false }], next: 'discord' }),
            moduleManifest: vi.fn().mockResolvedValue({ ok: true, manifest: { modules: { minecraft: { display_name: 'Minecraft' } } } }),
        } as unknown as typeof window.api;
        useUIStore.getState()._resetForTest();
        useSettingsStore.setState({ settingsReady: true, closedGuides: [] });
        useGuideStore.setState({ onboarding: null, step: null, addServerMode: null });
        useServerStore.setState({ modules: [] });
    });

    it('서버가 없으면 디스코드 연결부터, 나중에 하기를 누르면 불러오기·새로 만들기를 고른다', async () => {
        render(page());
        await act(async () => {});
        expect(screen.getByText("Let's connect Discord first.")).toBeInTheDocument();
        fireEvent.click(screen.getByRole('button', { name: 'Later' }));
        expect(screen.getByText('Do you already run a game server?')).toBeInTheDocument();
    });

    it('디스코드 연결하기는 디스코드 화면으로 보내고, 돌아오면 갈림길이다', async () => {
        render(page());
        await act(async () => {});
        fireEvent.click(screen.getByRole('button', { name: /Connect Discord/ }));
        expect(useUIStore.getState().activeTab).toBe('discord');
        expect(useGuideStore.getState().step).toBe('fork');
    });

    it('모듈이 없으면 불러오기 전에 모듈 받기를, 있으면 불러오기 창을 바로 연다', async () => {
        useGuideStore.setState({ step: 'fork' });
        const { rerender } = render(page());
        fireEvent.click(screen.getByRole('button', { name: /Import an existing server/ }));
        expect(useGuideStore.getState()).toMatchObject({ step: 'modules', after: 'migrate' });
        expect(await screen.findByText('Minecraft')).toBeInTheDocument();

        act(() => {
            useGuideStore.setState({ step: 'fork' });
            useServerStore.setState({ modules: [{ name: 'minecraft' }] as never });
        });
        rerender(page([{ name: 'minecraft' }]));
        fireEvent.click(screen.getByRole('button', { name: /Import an existing server/ }));
        expect(useUIStore.getState().showModuleManager).toBe(true);
        expect(useGuideStore.getState().addServerMode).toBe('migrate');
    });

    it('✕로 닫으면 빈 화면으로 돌아가고, 설정에서 다시 열 수 있다', async () => {
        render(page());
        await act(async () => {});
        fireEvent.click(screen.getAllByRole('button', { name: 'Close' })[0]);
        expect(useSettingsStore.getState().closedGuides).toContain('firstRun');
        expect(screen.queryByRole('button', { name: 'Later' })).toBeNull();
        act(() => openGuide('firstRun'));
        expect(useSettingsStore.getState().closedGuides).not.toContain('firstRun');
        expect(screen.getByRole('button', { name: 'Later' })).toBeInTheDocument();
    });
});
