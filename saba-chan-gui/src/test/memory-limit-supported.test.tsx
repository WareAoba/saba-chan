/**
 * 메모리 상한을 걸 수 없는 PC (데몬이 서버 정보에 memory_limit_supported:false 를 싣는다).
 * 사바쨩 탭의 메모리 상한 칸을 잠그고 이유를 알린다. 걸리지 않는 예전 값이 남았으면 알리고 "비우기"로 지운다.
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ServerSettingsModal } from '../components/ServerSettingsModal';
import type { Server } from '../stores/useServerStore';
import { setupSettingsStore } from './serverStore';
import i18n from '../i18n';

const t = (k: string) => i18n.t(`gui:${k}`) as string;
const modules = [{ name: 'game', settings: { fields: [{ name: 'title', field_type: 'text' }] } }];

beforeEach(() => {
    window.showToast = vi.fn();
    window.api = {
        serverList: vi.fn().mockResolvedValue(null),
        instanceReadLiveSettings: vi.fn().mockResolvedValue({ success: true, settings: { title: 'x' } }),
        instanceUpdateSettings: vi.fn().mockResolvedValue({ success: true }),
    } as unknown as typeof window.api;
});

/** 데몬이 준 서버 정보로 설정을 열고 사바쨩 탭을 그린다 */
async function open(server: Partial<Server>) {
    const { result } = setupSettingsStore({
        servers: [],
        modules: modules as never,
        setModal: vi.fn(),
        setProgressBar: vi.fn(),
        fetchServers: vi.fn(),
    });
    await act(async () => result.current.handleOpenSettings({ id: 's', name: 'S', module: 'game', status: 'stopped', ...server } as Server));
    const props = {
        ...result.current,
        settingsSource: 'live',
        settingsActiveTab: 'general',
        setSettingsActiveTab: vi.fn(),
        modules,
        servers: [],
        section: 'saba',
    };
    render(<ServerSettingsModal {...(props as unknown as ComponentProps<typeof ServerSettingsModal>)} />);
    return screen.getByRole('textbox', { name: t('memory_limit.label') }) as HTMLInputElement;
}

describe('메모리 상한을 걸 수 없는 PC', () => {
    it('칸을 잠그고 이유를 한 줄로 알리며, "Windows에서만 적용돼요"는 빼고 보인다', async () => {
        const input = await open({ memory_limit_mb: null, memory_limit_supported: false });
        expect(input.disabled).toBe(true);
        expect(screen.getByText(t('memory_limit.locked'))).toBeTruthy();
        expect(screen.getByText(t('memory_limit.hint_locked'))).toBeTruthy();
        expect(screen.queryByText(t('memory_limit.hint'))).toBeNull();
        expect(screen.queryByRole('button', { name: t('memory_limit.clear') })).toBeNull();
    });

    it('예전에 저장된 값이 남았으면 알리고, "비우기"는 빈 상한을 저장한다', async () => {
        const input = await open({ memory_limit_mb: 4096, memory_limit_supported: false });
        expect(input.disabled).toBe(true);
        expect(input.value).toBe('4096');
        expect(screen.getByText(t('memory_limit.stale'))).toBeTruthy();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: t('memory_limit.clear') })));
        expect(vi.mocked(window.api.instanceUpdateSettings).mock.calls[0][1]).toMatchObject({ memory_limit_mb: null });
    });

    it('값을 싣지 않는 예전 데몬은 걸 수 있는 것으로 본다 (지금처럼)', async () => {
        const input = await open({ memory_limit_mb: 4096 });
        expect(input.disabled).toBe(false);
        expect(screen.getByText(t('memory_limit.hint'))).toBeTruthy();
        expect(screen.queryByText(t('memory_limit.locked'))).toBeNull();
        expect(screen.queryByText(t('memory_limit.stale'))).toBeNull();
    });
});
