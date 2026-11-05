import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import i18n from 'i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { SettingsPanel } from '../components/SettingsPanel';
import { initI18n } from '../i18n';
import { useApp } from '../store';
import type { Module } from '../types';
import { asServer, mockApi } from './mocks';

// 데몬은 설정을 저장했지만 모듈이 게임 설정 파일에 쓰지 못하면 synced:false를 준다 (GUI useServerSettings와 같게 알린다)
const moduleData = { settings: { fields: [{ name: 'motd', field_type: 'text' }] } };
const server = { id: 's1', name: 'survival', module: 'mc', revision: 'r1', module_settings: { motd: 'old' } };

describe('웹 서버 설정 저장의 게임 파일 반영', () => {
    beforeAll(() => initI18n('en'));
    afterEach(() => {
        cleanup();
        useApp.setState({ confirm: null, toasts: [] });
    });

    it('게임 설정 파일에 쓰지 못했거나 쓰지 않는 키가 있으면 알린다', async () => {
        const api = {
            instanceUpdateSettings: vi.fn().mockResolvedValue({
                success: true,
                revision: 'r2',
                synced: false,
                sync_error: 'read-only file',
                ignored: ['rcon_port'],
            }),
        };
        render(
            <SettingsPanel
                api={mockApi(api)}
                moduleData={moduleData as unknown as Module}
                running={false}
                server={asServer(server)}
            />,
        );
        fireEvent.change(screen.getByRole('textbox', { name: /motd/ }), { target: { value: 'new' } });
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await act(async () => {});
        const toasts = useApp.getState().toasts.map((x) => [x.message, x.tone]);
        expect(toasts).toContainEqual([i18n.t('gui:settings.sync_failed', { error: 'read-only file' }), 'error']);
        expect(toasts).toContainEqual([i18n.t('gui:settings.ignored', { keys: 'rcon_port' }), 'info']);
        expect(toasts.some(([, tone]) => tone === 'ok')).toBe(false);
    });
});
