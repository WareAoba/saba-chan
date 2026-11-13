import type { ComponentProps, ComponentType } from 'react';
import { act, render } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../components/ExtensionSlot', () => ({ default: () => null, SlotView: () => null, useSlotPages: () => [] }));

import { ServerRow as ServerRowFull } from '../components/servers/ServerRow';
import { setupSettingsStore } from './serverStore';
import { unsavedServerIds } from '../stores/useServerSettingsStore';
import { useServerStore } from '../stores/useServerStore';

const ServerRow = ServerRowFull as unknown as ComponentType<Partial<ComponentProps<typeof ServerRowFull>>>;

// 설정을 고치다 다른 서버를 눌러도 고친 값은 메모리에 남고, 그 서버 카드에 경고 표시가 붙는다 (사용자 11-12)
const modules = [{ name: 'dl', settings: { fields: [{ name: 'motd', field_type: 'text' }] } }];
const srvA = { id: 'a', name: 'A', module: 'dl', status: 'stopped', module_settings: { motd: 'a-saved' } };
const srvB = { id: 'b', name: 'B', module: 'dl', status: 'stopped', module_settings: { motd: 'b-saved' } };

const setup = () =>
    setupSettingsStore({
        servers: [srvA, srvB] as never,
        modules: modules as never,
        setModal: vi.fn(),
        setProgressBar: vi.fn(),
        fetchServers: vi.fn(),
    });

describe('저장하지 않은 서버 설정', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = {
            serverList: vi.fn().mockResolvedValue({ servers: [srvA, srvB] }),
            instanceReadLiveSettings: vi.fn(async (id: string) => ({
                success: true,
                settings: { motd: id === 'a' ? 'a-saved' : 'b-saved' },
            })),
            instanceUpdateSettings: vi.fn().mockResolvedValue({ success: true }),
        } as unknown as typeof window.api;
    });

    it('다른 서버를 열었다 돌아오면 고치던 값이 그대로이고, 저장하면 그 값이 간다', async () => {
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings(srvA as never));
        expect(unsavedServerIds(result.current, modules as never)).toEqual([]);
        act(() => result.current.handleSettingChange('motd', 'a-edited'));
        expect(unsavedServerIds(result.current, modules as never)).toEqual(['a']);

        await act(async () => result.current.handleOpenSettings(srvB as never));
        expect(result.current.settingsValues.motd).toBe('b-saved');
        expect(unsavedServerIds(result.current, modules as never)).toEqual(['a']);

        await act(async () => result.current.handleOpenSettings(srvA as never));
        expect(result.current.settingsValues.motd).toBe('a-edited');
        expect(result.current.openedSettings.motd).toBe('a-saved');
        expect(unsavedServerIds(result.current, modules as never)).toEqual(['a']);

        await act(async () => result.current.handleSaveSettings());
        expect(window.api.instanceUpdateSettings).toHaveBeenCalledWith('a', expect.objectContaining({ motd: 'a-edited' }));
    });

    it('되돌리면 표시가 사라지고, 다른 서버에 다녀와도 돌아오지 않는다', async () => {
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings(srvA as never));
        act(() => result.current.handleSettingChange('motd', 'a-edited'));
        act(() => result.current.revertSettings());
        expect(unsavedServerIds(result.current, modules as never)).toEqual([]);
        await act(async () => result.current.handleOpenSettings(srvB as never));
        await act(async () => result.current.handleOpenSettings(srvA as never));
        expect(result.current.settingsValues.motd).toBe('a-saved');
        expect(unsavedServerIds(result.current, modules as never)).toEqual([]);
    });

    it('서버 카드에 경고 표시를 붙인다', () => {
        useServerStore.setState({ modulesLoaded: true });
        const row = (unsaved: boolean) =>
            render(
                <ServerRow
                    server={srvA as never}
                    index={0}
                    moduleData={{ name: 'dl' } as never}
                    unsaved={unsaved}
                    onSelect={() => {}}
                    onStart={() => {}}
                    onStop={() => {}}
                    onPointerDown={() => {}}
                    skipNextClick={{ current: false }}
                />,
            );
        const { unmount } = row(false);
        expect(document.querySelector('.server-row-flag.tone-warn')).toBeNull();
        unmount();
        row(true);
        expect(document.querySelector('.server-row-flag.tone-warn')!.getAttribute('aria-label')).toBe(
            'Settings have unsaved changes',
        );
    });
});
