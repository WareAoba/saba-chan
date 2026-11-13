import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerSettingsStore } from '../stores/useServerSettingsStore';
import { setupSettingsStore } from './serverStore';

// 저장한 뒤에도 설정 화면은 그대로 있다: 닫았다 다시 열면 탭 전체가 로딩 표시로 바뀌고 스크롤이 맨 위로 간다.
// 값·기준·revision은 그 자리에서 다시 읽는다
const modules = [
    {
        name: 'game',
        settings: {
            fields: [
                { name: 'title', field_type: 'text' },
                { name: 'motd', field_type: 'text' },
            ],
        },
    },
];
const server = { id: 's', name: 'S', module: 'game', revision: 'r1', module_settings: { title: 'old', motd: 'hi' }, stored: { title: 'old', motd: 'hi' } };

function setup() {
    const shown: boolean[] = [];
    setupSettingsStore({ modules });
    const hook = renderHook(() => {
        const settings = useServerSettingsStore();
        shown.push(settings.showSettingsModal);
        return settings;
    });
    return { ...hook, shown };
}

let saved: typeof server;
let release: (() => void) | null;
let holdList: boolean;

beforeEach(() => {
    window.showToast = vi.fn();
    saved = server;
    release = null;
    holdList = false;
    window.api = {
        serverList: vi.fn(async () => {
            if (holdList) await new Promise<void>((r) => (release = r));
            return { servers: [saved] };
        }),
        instanceReadLiveSettings: vi.fn(async () => ({
            success: true,
            revision: saved.revision,
            settings: saved.module_settings,
        })),
        instanceUpdateSettings: vi.fn(async (_id: string, body: Record<string, string>) => {
            saved = { ...saved, revision: 'r2', module_settings: { ...saved.module_settings, title: body.title }, stored: { ...saved.stored, title: body.title } };
            return { success: true, synced: true };
        }),
    } as unknown as typeof window.api;
});

describe('서버 설정 저장 뒤', () => {
    it('화면을 닫지 않고 그 자리에서 기준 값과 revision을 다시 읽는다', async () => {
        const { result, shown } = setup();
        await act(async () => result.current.handleOpenSettings(server as never));
        act(() => result.current.setAdvancedExpanded(true));
        act(() => result.current.handleSettingChange('title', 'new'));
        const from = shown.length;
        await act(async () => result.current.handleSaveSettings());

        expect(shown.slice(from)).not.toContain(false);
        expect(result.current.advancedExpanded).toBe(true);
        expect(result.current.settingsValues.title).toBe('new');
        expect(result.current.openedSettings.title).toBe('new');

        // 다음 저장은 새 revision으로, 이미 저장한 칸은 다시 보내지 않는다
        act(() => result.current.handleSettingChange('motd', 'bye'));
        await act(async () => result.current.handleSaveSettings());
        const sent = (window.api.instanceUpdateSettings as ReturnType<typeof vi.fn>).mock.calls[1][1];
        expect(sent.revision).toBe('r2');
        expect(sent.motd).toBe('bye');
        expect(sent.title).toBeUndefined();
    });

    it('저장한 뒤 다시 읽기에 실패해도 저장한 값과 새 revision을 그대로 둔다', async () => {
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings(server as never));
        act(() => result.current.handleSettingChange('title', 'new'));
        window.api.instanceUpdateSettings = vi.fn(async () => ({ success: true, synced: true, revision: 'r2' })) as never;
        window.api.serverList = vi.fn(async () => {
            throw new Error('network');
        }) as never;
        await act(async () => result.current.handleSaveSettings());

        expect(result.current.settingsValues.title).toBe('new');
        expect(result.current.openedSettings.title).toBe('new');

        act(() => result.current.handleSettingChange('motd', 'bye'));
        await act(async () => result.current.handleSaveSettings());
        const sent = (window.api.instanceUpdateSettings as ReturnType<typeof vi.fn>).mock.calls[1][1];
        expect(sent.revision).toBe('r2');
        expect(sent.motd).toBe('bye');
    });

    it('다시 읽는 사이 고친 칸은 지우지 않는다', async () => {
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings(server as never));
        act(() => result.current.handleSettingChange('title', 'new'));
        holdList = true;
        let saving: Promise<void> = Promise.resolve();
        await act(async () => {
            saving = result.current.handleSaveSettings();
            await vi.waitFor(() => expect(release).not.toBeNull());
        });
        act(() => result.current.handleSettingChange('motd', 'typed while reloading'));
        await act(async () => {
            release?.();
            await saving;
        });
        expect(result.current.settingsValues.motd).toBe('typed while reloading');
        expect(result.current.openedSettings.motd).toBe('hi');
    });

    it('버전 목록이 오기 전에 저장해도 버전 목록을 끝까지 받는다', async () => {
        const game = { ...modules[0], install: { method: 'download' }, functions: ['list_versions', 'install_server'] };
        const { result } = setupSettingsStore({
            servers: [],
            modules: [game] as never,
            setModal: vi.fn(),
            setProgressBar: vi.fn(),
            fetchServers: vi.fn(),
        });
        // 처음 열 때의 버전 목록은 저장이 끝난 뒤에야 온다
        const arrive: Array<() => void> = [];
        let calls = 0;
        window.api.moduleListVersions = vi.fn(async () => {
            if (calls++ === 0) await new Promise<void>((r) => arrive.push(r));
            return { versions: [{ id: '1.0' }, { id: '1.1' }] };
        }) as never;

        let opening: Promise<void> = Promise.resolve();
        await act(async () => {
            opening = result.current.handleOpenSettings(server as never);
            await vi.waitFor(() => expect(arrive).toHaveLength(1));
        });
        act(() => result.current.handleSettingChange('title', 'new'));
        await act(async () => result.current.handleSaveSettings());
        await act(async () => {
            arrive[0]();
            await opening;
        });

        expect(result.current.versionsLoading).toBe(false);
        expect(result.current.availableVersions).toHaveLength(2);
    });
});
