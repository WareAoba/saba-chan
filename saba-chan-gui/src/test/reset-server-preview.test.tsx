/**
 * 서버 초기화 확인 창: 데몬이 알려 준(GET …/server/reset) 지울 항목을 그대로 보인다.
 * 모듈이 범위를 둘 이상 밝히면 고르게 하고, 고른 범위와 "지우기 전에 백업"을 초기화에 실어 보낸다.
 * 미리 보기를 주지 않는 모듈은 예전 확인 문구로 묻는다.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import i18n from 'i18next';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ServerSettingsModal } from '../components/ServerSettingsModal';
import type { Server } from '../stores/useServerStore';
import type { ModuleInfo } from '../types/daemon';

const tr = (key: string, vars?: Record<string, unknown>) => i18n.t(`gui:server_settings.${key}`, vars ?? {});
const module = { name: 'dl', settings: { fields: [] }, functions: null } as unknown as ModuleInfo;
const srv = { id: 's1', name: '친구들 서버', module: 'dl', status: 'stopped', module_settings: {} } as unknown as Server;

const mount = () => {
    const handleResetServer = vi.fn();
    render(
        <ServerSettingsModal
            {...({
                settingsServer: srv,
                settingsValues: {},
                settingsSource: 'live',
                settingsActiveTab: 'general',
                setSettingsActiveTab: vi.fn(),
                modules: [module],
                advancedExpanded: true,
                setAdvancedExpanded: vi.fn(),
                availableVersions: [],
                handleSettingChange: vi.fn(),
                handleInstallVersion: vi.fn(),
                handleSaveSettings: vi.fn(),
                handleResetServer,
                resettingServer: false,
                onClose: vi.fn(),
                servers: [srv],
            } as unknown as Parameters<typeof ServerSettingsModal>[0])}
        />,
    );
    fireEvent.click(document.querySelector('.ss-reset') as HTMLElement);
    return handleResetServer;
};

const WORLD = { label: '월드', paths: ['world/', 'world_nether/'], size: 2048 };
const TWO_SCOPES = {
    success: true,
    scopes: [
        { id: 'world', label: '월드만', backup: true, items: [WORLD] },
        { id: 'all', label: '전체', backup: true, items: [WORLD, { label: '게임 설정', paths: ['server.properties'], size: 1024 }] },
    ],
};

afterEach(() => {
    delete (window.api as unknown as Record<string, unknown>).instanceResetPreview;
});

describe('서버 초기화 확인 창', () => {
    it('범위마다 지울 항목과 크기를 보이고, 고른 범위와 백업 여부를 초기화에 싣는다', async () => {
        window.api.instanceResetPreview = vi.fn().mockResolvedValue(TWO_SCOPES);
        const handleResetServer = mount();
        const dialog = await screen.findByRole('dialog');
        expect(window.api.instanceResetPreview).toHaveBeenCalledWith('s1');
        expect(dialog.textContent).toContain(tr('reset_preview_desc', { name: '친구들 서버' }));
        // 좁은 범위가 먼저 골라져 있다
        expect(screen.getByRole('radio', { name: '월드만' }).getAttribute('aria-checked')).toBe('true');
        expect(dialog.textContent).toContain('world/ · world_nether/');
        expect(dialog.textContent).toContain('2.0 KB');
        expect(dialog.textContent).not.toContain('server.properties');
        expect(screen.getByRole('button', { name: tr('reset_delete_count', { count: 1 }) })).toBeTruthy();

        fireEvent.click(screen.getByRole('radio', { name: '전체' }));
        expect(dialog.textContent).toContain('server.properties');
        expect((screen.getByRole('checkbox', { name: tr('reset_backup_first') }) as HTMLInputElement).checked).toBe(true);
        fireEvent.click(screen.getByRole('button', { name: tr('reset_delete_count', { count: 2 }) }));
        expect(handleResetServer).toHaveBeenCalledWith({ scope: 'all', backup: true });
        await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    });

    it('범위가 하나면 고르기 없이 목록과 안내만 보이고, 백업이 담지 않는 것이면 백업을 묻지 않는다', async () => {
        window.api.instanceResetPreview = vi.fn().mockResolvedValue({
            success: true,
            scopes: [{ id: 'settings', backup: false, note: '월드는 그대로 남아요.', items: [{ label: '게임 설정을 기본값으로 되돌려요', paths: ['Pal/Saved/Config/PalWorldSettings.ini'], size: 6144 }] }],
        });
        const handleResetServer = mount();
        const dialog = await screen.findByRole('dialog');
        expect(screen.queryByRole('radiogroup')).toBeNull();
        expect(screen.queryByRole('checkbox')).toBeNull();
        expect(dialog.textContent).toContain('월드는 그대로 남아요.');
        fireEvent.click(screen.getByRole('button', { name: tr('reset_delete_count', { count: 1 }) }));
        expect(handleResetServer).toHaveBeenCalledWith({ scope: 'settings', backup: false });
    });

    it('미리 보기를 주지 않는 모듈은 예전 확인으로 넘어간다', async () => {
        window.api.instanceResetPreview = vi.fn().mockResolvedValue({ success: false, error_code: 'not_supported' });
        const handleResetServer = mount();
        await waitFor(() => expect(handleResetServer).toHaveBeenCalledWith());
        expect(screen.queryByRole('dialog')).toBeNull();
    });
});
