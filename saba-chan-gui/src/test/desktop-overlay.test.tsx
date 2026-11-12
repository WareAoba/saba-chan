/**
 * 바탕화면 오버레이: 익스텐션 slot `Desktop.overlay`를 그리는 투명 창 (셸 overlay.rs).
 * 창은 셸이 띄우고, 이 화면은 slot을 그리고 클릭(앱 열기)·끌기(창 옮기기)·우클릭(서버 메뉴)을 맡는다.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { DesktopOverlay } from '../components/shell/DesktopOverlay';
import { ExtensionProvider } from '../contexts/ExtensionContext';
import { loadsOn, SLOT_TABLE } from '../extensions/slotTable';

const flush = () => act(async () => {});

describe('slot 표', () => {
    it('Desktop.overlay는 오버레이 창의 slot이고, 그 창은 그 slot을 선언한 번들만 읽는다', () => {
        expect(SLOT_TABLE['Desktop.overlay']).toEqual({ surface: 'desktop', kind: 'inline' });
        const gui = (slots: Record<string, unknown>) => ({
            gui: { bundle: 'x.js', styles: null, builtin: null, slots } as never,
        });
        expect(loadsOn(gui({ 'Desktop.overlay': {} }), 'desktop')).toBe(true);
        expect(loadsOn(gui({ 'Desktop.overlay': {} }), 'main')).toBe(false);
        expect(loadsOn(gui({ 'App.page': {} }), 'desktop')).toBe(false);
        expect(loadsOn(gui({}), 'desktop')).toBe(false);
    });
});

describe('오버레이 브리지', () => {
    it('셸 명령으로 간다', async () => {
        const invoke = vi.fn().mockResolvedValue({ ok: true });
        const on = vi.fn();
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn(), on } as never });
        await api.overlayFit(180, 220);
        expect(invoke).toHaveBeenLastCalledWith('overlay_fit', { width: 180, height: 220 });
        await api.overlayMenu();
        expect(invoke).toHaveBeenLastCalledWith('overlay_menu');
        await api.overlayMenuFit(212);
        expect(invoke).toHaveBeenLastCalledWith('overlay_menu_fit', { height: 212 });
        await api.overlaySetScale(150);
        expect(invoke).toHaveBeenLastCalledWith('overlay_set_scale', { scale: 150 });
        const listener = vi.fn();
        api.onOverlayScale(listener);
        expect(on).toHaveBeenLastCalledWith('overlay:scale', listener);
        await api.overlayAction('hide');
        expect(invoke).toHaveBeenLastCalledWith('overlay_action', { action: 'hide' });
        await api.overlayAction('open_app', 'server:b');
        expect(invoke).toHaveBeenLastCalledWith('overlay_action', { action: 'open_app', route: 'server:b' });
        await api.overlayStartDrag();
        expect(invoke).toHaveBeenLastCalledWith('overlay_drag');
        await api.overlaySetEnabled(false);
        expect(invoke).toHaveBeenLastCalledWith('overlay_set_enabled', { enabled: false });
        await api.overlaySync();
        expect(invoke).toHaveBeenLastCalledWith('overlay_sync');
    });
});

describe('오버레이 화면', () => {
    let scaleListener: ((scale: number) => void) | null = null;
    beforeEach(() => {
        window.api.daemonStatus = vi.fn().mockResolvedValue({ running: true });
        window.api.daemonTarget = vi.fn().mockResolvedValue({ target: null });
        window.api.serverList = vi
            .fn()
            .mockResolvedValue({ servers: [{ id: 'b', name: '팰월드', status: 'stopped' }] });
        window.api.instanceStart = vi.fn().mockResolvedValue({ success: true });
        window.api.instanceStop = vi.fn().mockResolvedValue({ success: true });
        window.api.overlayFit = vi.fn();
        window.api.overlayMenu = vi.fn().mockResolvedValue(true);
        window.api.overlayAction = vi.fn().mockResolvedValue({ ok: true });
        window.api.overlayStartDrag = vi.fn().mockResolvedValue(undefined);
        window.api.onOverlayScale = vi.fn((cb) => {
            scaleListener = cb;
            return () => {
                scaleListener = null;
                return true;
            };
        });
        window.api.extensionList = vi.fn().mockResolvedValue({
            extensions: [
                {
                    id: 'pet',
                    name: 'pet',
                    enabled: true,
                    version: '1.0.0',
                    gui: { bundle: 'x.js', styles: null, builtin: null, slots: { 'Desktop.overlay': {} } },
                    gui_allowed: true,
                    config_fields: {},
                },
            ],
        });
        window.api.extensionI18n = vi.fn().mockResolvedValue(null);
        window.SabaExtPet = { registerSlots: () => ({ 'Desktop.overlay': () => <div>펫이다</div> }) };
    });
    afterEach(() => {
        for (const k of [
            'daemonStatus',
            'daemonTarget',
            'serverList',
            'instanceStart',
            'instanceStop',
            'overlayFit',
            'overlayMenu',
            'overlayAction',
            'overlayStartDrag',
            'onOverlayScale',
            'extensionList',
            'extensionI18n',
        ])
            delete (window.api as unknown as Record<string, unknown>)[k];
        delete window.SabaExtPet;
        window.history.replaceState(null, '', '/');
        vi.restoreAllMocks();
    });

    const mount = async () => {
        render(
            <ExtensionProvider surface="desktop">
                <DesktopOverlay />
            </ExtensionProvider>,
        );
        await waitFor(() => expect(screen.getByText('펫이다')).toBeTruthy());
        await flush();
        return document.querySelector('.desktop-overlay') as HTMLElement;
    };

    it('slot을 그리고 크기를 셸에 알린다', async () => {
        await mount();
        expect(window.api.overlayFit).toHaveBeenCalled();
    });

    it('원격 데몬을 골라도 번들을 주는 이 PC 데몬의 익스텐션 목록을 읽는다', async () => {
        await mount();
        expect(window.api.extensionList).toHaveBeenCalledWith({ local: true });
    });

    it('누르면 앱을 열고, 누른 채 움직이면 창을 끌 뿐 앱을 열지 않는다', async () => {
        const root = await mount();
        fireEvent.pointerDown(root, { button: 0, screenX: 10, screenY: 10 });
        fireEvent.pointerUp(root, { button: 0, screenX: 11, screenY: 10 });
        expect(window.api.overlayAction).toHaveBeenCalledWith('open_app');
        vi.mocked(window.api.overlayAction).mockClear();

        fireEvent.pointerDown(root, { button: 0, screenX: 10, screenY: 10 });
        fireEvent.pointerMove(root, { buttons: 1, screenX: 30, screenY: 10 });
        fireEvent.pointerUp(root, { button: 0, screenX: 30, screenY: 10 });
        expect(window.api.overlayStartDrag).toHaveBeenCalledTimes(1);
        expect(window.api.overlayAction).not.toHaveBeenCalled();
    });

    it('익스텐션이 고른 화면(서버)을 앱에서 바로 연다. 모르는 화면은 넘기지 않는다', async () => {
        const got: { host?: { openApp: (route?: string) => void } } = {};
        window.SabaExtPet = {
            registerSlots: () => ({
                'Desktop.overlay': (props: { host: { openApp: (route?: string) => void } }) => {
                    got.host = props.host;
                    return <div>펫이다</div>;
                },
            }),
        };
        await mount();
        got.host?.openApp('server:b');
        expect(window.api.overlayAction).toHaveBeenLastCalledWith('open_app', 'server:b');
        vi.mocked(window.api.overlayAction).mockClear();
        got.host?.openApp('settings');
        expect(window.api.overlayAction).toHaveBeenLastCalledWith('open_app', 'settings');
        vi.mocked(window.api.overlayAction).mockClear();
        got.host?.openApp('../../etc');
        expect(window.api.overlayAction).toHaveBeenCalledWith('open_app');
    });

    it('우클릭은 셸에 펫 옆의 판을 열게 한다', async () => {
        const root = await mount();
        fireEvent.contextMenu(root);
        expect(window.api.overlayMenu).toHaveBeenCalledWith();
    });

    it('펫 크기: 주소의 처음 값으로 그리고, 셸이 알리면 바로 바꾼다', async () => {
        window.history.replaceState(null, '', '/?desktop-overlay=1&scale=150');
        const root = await mount();
        const zoomed = root.querySelector('.desktop-overlay-zoom') as HTMLElement;
        expect(zoomed.style.zoom).toBe('1.5');
        await act(async () => scaleListener?.(80));
        expect(zoomed.style.zoom).toBe('0.8');
    });
});

describe('메인 창과 셸', () => {
    afterEach(() => {
        for (const k of ['extensionList', 'extensionI18n', 'overlaySync', 'overlayState', 'overlaySetEnabled'])
            delete (window.api as unknown as Record<string, unknown>)[k];
        vi.restoreAllMocks();
    });

    it('오버레이 slot을 쓰는 익스텐션을 켜고 끄면 셸에 다시 보게 한다 (처음 목록은 셸이 이미 봤다)', async () => {
        const pet = (enabled: boolean) => ({
            id: 'pet',
            name: 'pet',
            enabled,
            version: '1.0.0',
            gui: { bundle: 'x.js', styles: null, builtin: null, slots: { 'Desktop.overlay': {} } },
            gui_allowed: false,
            config_fields: {},
        });
        window.api.extensionList = vi.fn().mockResolvedValue({ extensions: [pet(false)] });
        window.api.extensionI18n = vi.fn().mockResolvedValue(null);
        window.api.overlaySync = vi.fn();
        vi.spyOn(console, 'warn').mockImplementation(() => {});
        const { useExtensions } = await import('../contexts/ExtensionContext');
        const { renderHook } = await import('@testing-library/react');
        const { result } = renderHook(() => useExtensions(), { wrapper: ExtensionProvider });
        await waitFor(() => expect(result.current.loading).toBe(false));
        expect(window.api.overlaySync).not.toHaveBeenCalled();
        window.api.extensionList = vi.fn().mockResolvedValue({ extensions: [pet(true)] });
        await act(async () => {
            await result.current.refreshExtensions();
        });
        await waitFor(() => expect(window.api.overlaySync).toHaveBeenCalledTimes(1));
    });

    it('설정 스위치는 그 slot을 쓰는 익스텐션이 있을 때만 보이고 셸에 저장한다', async () => {
        const { DesktopOverlaySetting } = await import('../components/Modals/DesktopOverlaySetting');
        window.api.overlayState = vi.fn().mockResolvedValue({ enabled: true, available: false, supported: true });
        const { container, unmount } = render(<DesktopOverlaySetting />);
        await flush();
        expect(container.innerHTML).toBe('');
        unmount();
        window.api.overlayState = vi.fn().mockResolvedValue({ enabled: true, available: true, supported: true });
        window.api.overlaySetEnabled = vi.fn().mockResolvedValue({ ok: true });
        render(<DesktopOverlaySetting />);
        await flush();
        fireEvent.click(screen.getByRole('checkbox'));
        await flush();
        expect(window.api.overlaySetEnabled).toHaveBeenCalledWith(false);
    });
});

describe('허락', () => {
    afterEach(() => {
        for (const k of ['extensionList', 'extensionI18n', 'extensionGuiBundle', 'overlaySync', 'daemonTarget'])
            delete (window.api as unknown as Record<string, unknown>)[k];
        vi.restoreAllMocks();
    });

    it('바탕화면에만 그리는 번들도 허락은 메인 창이 묻고, 번들은 메인 창에서 읽지 않는다', async () => {
        window.api.extensionList = vi.fn().mockResolvedValue({
            extensions: [
                {
                    id: 'pet',
                    name: 'pet',
                    enabled: true,
                    version: '1.0.0',
                    gui: { bundle: 'x.js', styles: null, builtin: null, slots: { 'Desktop.overlay': {} } },
                    gui_allowed: false,
                    config_fields: {},
                },
            ],
        });
        window.api.extensionI18n = vi.fn().mockResolvedValue(null);
        window.api.extensionGuiBundle = vi.fn().mockResolvedValue(null);
        const { useExtensions } = await import('../contexts/ExtensionContext');
        const { renderHook } = await import('@testing-library/react');
        const { result } = renderHook(() => useExtensions(), { wrapper: ExtensionProvider });
        await waitFor(() => expect(result.current.guiPending.map((e) => e.id)).toEqual(['pet']));
        expect(window.api.extensionGuiBundle).not.toHaveBeenCalled();
    });

    it('원격 데몬을 고른 동안에도 이 PC 데몬의 바탕화면 번들 허락을 묻는다 (메인 화면 slot도 함께 선언해도)', async () => {
        const pet = {
            id: 'pet',
            name: 'pet',
            enabled: true,
            version: '1.0.0',
            gui: { bundle: 'x.js', styles: null, builtin: null, slots: { 'Desktop.overlay': {}, 'App.page': {} } },
            gui_allowed: false,
            config_fields: {},
        };
        // 고른 원격 데몬에는 펫이 없고, 이 PC 데몬에만 있다
        window.api.extensionList = vi.fn((opts?: { local?: boolean }) =>
            Promise.resolve({ extensions: opts?.local ? [pet] : [] }),
        ) as unknown as typeof window.api.extensionList;
        window.api.daemonTarget = vi.fn().mockResolvedValue({ target: 'remote-pc' });
        window.api.extensionI18n = vi.fn().mockResolvedValue(null);
        window.api.extensionGuiBundle = vi.fn().mockResolvedValue(null);
        const { useExtensions } = await import('../contexts/ExtensionContext');
        const { renderHook } = await import('@testing-library/react');
        const { result } = renderHook(() => useExtensions(), { wrapper: ExtensionProvider });
        await waitFor(() => expect(result.current.guiPending.map((e) => e.id)).toEqual(['pet']));
        expect(window.api.extensionGuiBundle).not.toHaveBeenCalled();
    });
});
