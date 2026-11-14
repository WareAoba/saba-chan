import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ComponentType } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useGuideStore } from '../components/guides/guides';
import { ServersPage as ServersPageImpl } from '../components/servers/ServersPage';
import { useServerStore } from '../stores/useServerStore';
import { useSettingsStore } from '../stores/useSettingsStore';
import { useUIStore } from '../stores/useUIStore';

// 서버가 생긴 뒤의 상세는 이 테스트의 관심 밖이다
vi.mock('../components/servers/ServerDetail', () => ({ ServerDetail: () => null }));

const ServersPage = ServersPageImpl as unknown as ComponentType<Record<string, unknown>>;
const page = (modules: { name: string }[] = []) => (
    <ServersPage servers={[]} modules={modules} onAddServer={vi.fn()} drag={{ cardRefs: { current: {} } }} />
);

// 사용자 11-08~10: 처음 켜면 디스코드 연결 → 쓰던 서버 불러오기 / 새로 만들기, 모듈이 없으면 모듈 받기를 먼저
describe('처음 시작하기 가이드', () => {
    beforeEach(() => {
        window.api = {
            onboarding: vi.fn().mockResolvedValue({ steps: [{ id: 'discord', done: false }], next: 'discord' }),
            moduleManifest: vi.fn().mockResolvedValue({ success: true, manifest: { modules: { minecraft: { display_name: 'Minecraft' } } } }),
        } as unknown as typeof window.api;
        useUIStore.getState()._resetForTest();
        useSettingsStore.setState({ settingsReady: true, closedGuides: [] });
        useGuideStore.setState({ onboarding: null, step: null, addServerMode: null });
        useServerStore.setState({ modules: [] });
    });

    it('서버가 없으면 디스코드 연결부터, 나중에 하기를 누르면 불러오기·새로 만들기를 고른다', async () => {
        render(page());
        await act(async () => {});
        expect(screen.getByText("Connect Discord first.")).toBeInTheDocument();
        fireEvent.click(screen.getByRole('button', { name: 'Later' }));
        expect(screen.getByText('Do you already run a game server?')).toBeInTheDocument();
    });

    it('가이드가 열릴 때 가이드 열림음을 한 번 내고, 단계를 넘길 때는 다시 내지 않는다', async () => {
        const playSound = vi.fn();
        (window.api as unknown as Record<string, unknown>).playSound = playSound;
        render(page());
        await act(async () => {});
        fireEvent.click(screen.getByRole('button', { name: 'Later' }));
        expect(playSound).toHaveBeenCalledTimes(1);
        expect(playSound).toHaveBeenCalledWith('guideOpen');
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

    it('모듈을 받으면 체크리스트의 다음 할 일도 다시 묻는다', async () => {
        const api = window.api as unknown as Record<string, ReturnType<typeof vi.fn>>;
        api.moduleInstallFromManifest = vi.fn().mockResolvedValue({ success: true });
        useServerStore.setState({ fetchModules: vi.fn().mockResolvedValue(undefined) });
        useGuideStore.setState({ step: 'modules', after: null });
        render(page());
        await act(async () => {});
        const before = api.onboarding.mock.calls.length;
        fireEvent.click(await screen.findByRole('button', { name: 'Get' }));
        await waitFor(() => expect(api.onboarding.mock.calls.length).toBe(before + 1));
    });

    it('갈림길에 머문 채 서버가 생기면 가이드를 접고 서버 목록을 보인다', async () => {
        useGuideStore.setState({ step: 'fork' });
        const { rerender } = render(page());
        expect(screen.getByText('Do you already run a game server?')).toBeInTheDocument();
        const server = { id: 's1', name: 'Alpha', module: 'minecraft', status: 'stopped' };
        rerender(<ServersPage servers={[server]} modules={[]} onAddServer={vi.fn()} drag={{ cardRefs: { current: {} } }} />);
        await act(async () => {});
        expect(useGuideStore.getState().step).toBeNull();
        expect(screen.queryByText('Do you already run a game server?')).toBeNull();
    });

    it('✕로 닫으면 빈 화면으로 돌아간다', async () => {
        render(page());
        await act(async () => {});
        fireEvent.click(screen.getAllByRole('button', { name: 'Close' })[0]);
        expect(useSettingsStore.getState().closedGuides).toContain('firstRun');
        expect(screen.queryByRole('button', { name: 'Later' })).toBeNull();
    });
});
