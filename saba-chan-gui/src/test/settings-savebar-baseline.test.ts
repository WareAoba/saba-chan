import { act } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { setupSettingsStore } from './serverStore';

// 저장 막대의 기준(연 때 값)은 설정 훅이 쥔다: 캐시로 연 뒤 실제 파일 값이 오면 그 값이 기준이고,
// 되돌리기는 그 기준으로 돌아가며 고친 칸 표시도 지운다 (캐시 값을 파일에 덮어쓰지 않게)
const modules = [{ name: 'game', settings: { fields: [{ name: 'title', field_type: 'text' }] } }];
const server = { id: 's', name: 'S', module: 'game', module_settings: { title: 'cached' } };

function setup() {
    return setupSettingsStore({ servers: [], modules: modules as never, setModal: vi.fn(), setProgressBar: vi.fn(), fetchServers: vi.fn() });
}

beforeEach(() => {
    window.showToast = vi.fn();
    window.api = {
        serverList: vi.fn().mockResolvedValue(null),
        instanceReadLiveSettings: vi.fn().mockResolvedValue({ success: true, settings: { title: 'live' } }),
        instanceUpdateSettings: vi.fn().mockResolvedValue({ success: true, synced: true }),
    } as unknown as typeof window.api;
});

describe('서버 설정 저장 막대 기준', () => {
    it('실제 파일 값이 오면 기준도 그 값이라, 손대지 않으면 바뀐 게 없다', async () => {
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings(server as never));
        expect(result.current.settingsValues.title).toBe('live');
        expect(result.current.openedSettings.title).toBe('live');
    });

    it('고친 뒤 되돌리면 실제 값으로 돌아가고, 저장해도 그 칸을 보내지 않는다', async () => {
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings(server as never));
        act(() => result.current.handleSettingChange('title', 'mine'));
        expect(result.current.settingsValues.title).toBe('mine');
        act(() => result.current.revertSettings());
        expect(result.current.settingsValues.title).toBe('live');
        await act(async () => result.current.handleSaveSettings());
        const sent = (window.api.instanceUpdateSettings as ReturnType<typeof vi.fn>).mock.calls[0]?.[1] ?? {};
        expect(sent.title === undefined || sent.title === 'live').toBe(true);
    });

    it('읽어 온 값(추가 실행 인자)은 바꾼 것이 아니다', async () => {
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings(server as never));
        act(() => result.current.handleSettingLoaded('_launch_extra_args', ['-x']));
        expect(result.current.openedSettings._launch_extra_args).toEqual(['-x']);
        expect(result.current.settingsValues._launch_extra_args).toEqual(['-x']);
    });
});
