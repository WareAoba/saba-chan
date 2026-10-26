import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { SettingsPanel } from '../components/SettingsPanel';
import { initI18n } from '../i18n';
import { useApp } from '../store';

// 감사 P2-3: 4초 폴링이 새 revision을 줘도 저장은 고치기 시작한 때의 revision을 싣는다 (GUI useServerSettings와 같다)
const moduleData = { settings: { fields: [{ name: 'motd', field_type: 'text' }] } };
const at = (revision, motd) => ({ id: 's1', name: 'survival', module: 'mc', revision, module_settings: { motd } });
const flush = () => act(async () => {});

describe('웹 서버 설정 저장의 revision', () => {
    beforeAll(() => initI18n('en'));
    afterEach(() => {
        cleanup();
        useApp.setState({ confirm: null, toasts: [] });
    });

    const mount = (api, server) => {
        const props = { api, moduleData, running: false, onDeleted: () => {}, control: { busy: false } };
        const view = render(<SettingsPanel {...props} server={server} />);
        return (next) => view.rerender(<SettingsPanel {...props} server={next} />);
    };
    const motd = () => screen.getByRole('textbox', { name: /motd/ });

    it('고치기 전에는 폴링을 따라가고, 고친 뒤에는 고치기 시작한 때의 revision을 싣는다', async () => {
        const api = { instanceUpdateSettings: vi.fn().mockResolvedValue({ success: true, revision: 'r4' }) };
        const poll = mount(api, at('r1', 'old'));
        poll(at('r2', 'from gui'));
        expect(motd().value).toBe('from gui');
        fireEvent.change(motd(), { target: { value: 'mine' } });
        poll(at('r3', 'from cli'));
        expect(motd().value).toBe('mine');
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await flush();
        expect(api.instanceUpdateSettings).toHaveBeenLastCalledWith('s1', { motd: 'mine', revision: 'r2' });
        // 저장이 준 revision으로 다음 저장을 잇는다
        fireEvent.change(motd(), { target: { value: 'mine 2' } });
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await flush();
        expect(api.instanceUpdateSettings).toHaveBeenLastCalledWith('s1', { motd: 'mine 2', revision: 'r4' });
    });

    it('다른 곳에서 바뀌었으면 GUI처럼 창으로 알리고, 고르면 데몬에서 지금 값을 다시 읽어 온다', async () => {
        const api = {
            instanceUpdateSettings: vi.fn().mockResolvedValue({
                success: false,
                error: 'changed',
                error_code: 'settings_changed',
                revision: 'r3',
            }),
            // 마지막 폴링(r2)보다 새 값: 다시 불러오기는 폴링이 아니라 데몬의 지금 값을 쓴다
            instanceGet: vi.fn().mockResolvedValue({ ...at('r3', 'from cli'), module: undefined, module_name: 'mc' }),
        };
        const poll = mount(api, at('r1', 'old'));
        fireEvent.change(motd(), { target: { value: 'mine' } });
        poll(at('r2', 'from gui'));
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await flush();
        const confirm = useApp.getState().confirm;
        expect(confirm?.message).toBe(
            'The settings were changed elsewhere in the meantime. Load the new values, then make your change again.',
        );
        expect(motd().value).toBe('mine');
        await act(async () => confirm.done(true));
        await flush();
        expect(api.instanceGet).toHaveBeenCalledWith('s1');
        expect(motd().value).toBe('from cli');
        fireEvent.change(motd(), { target: { value: 'again' } });
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await flush();
        expect(api.instanceUpdateSettings).toHaveBeenLastCalledWith('s1', { motd: 'again', revision: 'r3' });
    });

    it('지금 값을 읽지 못하면 고친 값을 그대로 두고 알린다', async () => {
        const api = {
            instanceUpdateSettings: vi.fn().mockResolvedValue({ success: false, error_code: 'settings_changed', revision: 'r3' }),
            instanceGet: vi.fn().mockResolvedValue({ ok: false, error: 'offline' }),
        };
        mount(api, at('r1', 'old'));
        fireEvent.change(motd(), { target: { value: 'mine' } });
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        await flush();
        await act(async () => useApp.getState().confirm.done(true));
        await flush();
        expect(motd().value).toBe('mine');
        expect(useApp.getState().toasts.length).toBeGreaterThan(0);
    });
});
