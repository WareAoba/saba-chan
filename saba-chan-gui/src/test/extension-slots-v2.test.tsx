/**
 * slot 표·manifest 선언·host 통로·탭형 slot (익스텐션 slot 확장 1단계, 사용자 2026-11-09).
 */
import { render, renderHook, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ExtensionSlot, { SlotView, useSlotPages } from '../components/ExtensionSlot';
import { ExtensionProvider, useExtensions } from '../contexts/ExtensionContext';
import { type ExtensionHost, HostWindowContext, isAppRoute } from '../extensions/host';
import { declaredSlots, loadsOn, SLOT_TABLE, slotProblems } from '../extensions/slotTable';

const gui = (slots: Record<string, unknown>, bundle: string | null = 'x.js') => ({ bundle, styles: null, builtin: null, slots });
const ext = (id: string, slots: Record<string, unknown>, extra: Record<string, unknown> = {}) => ({
    id,
    name: `${id} name`,
    enabled: true,
    version: '1.0.0',
    gui: gui(slots),
    gui_allowed: true,
    config_fields: {},
    ...extra,
});

describe('slot 표', () => {
    it('새 slot이 표에 있고 창이 정해져 있다', () => {
        for (const id of ['App.page', 'Settings.tab', 'AppBar.actions', 'AddServer.method', 'Discord.nav', 'ServerDetail.tab']) {
            expect(SLOT_TABLE[id as keyof typeof SLOT_TABLE].surface).toBe('main');
        }
        expect(SLOT_TABLE['TrayPopup.section'].surface).toBe('tray');
    });

    it('manifest gui.slots를 표시 정보로 읽는다 (옛 문자열 모양은 빈 정보)', () => {
        const d = declaredSlots({ gui: gui({ 'App.page': { label: 'page.title', icon: 'globe', x: 1 }, 'ServerCard.badge': 'Badge' }) as never });
        expect(d).toEqual({ 'App.page': { label: 'page.title', icon: 'globe' }, 'ServerCard.badge': {} });
    });

    it('모르는 slot과 선언 없는 등록을 짚는다', () => {
        const problems = slotProblems({ id: 'a', gui: gui({ 'Nope.slot': {} }) as never }, ['App.page']);
        expect(problems).toHaveLength(2);
        expect(problems.join(' ')).toContain('Nope.slot');
        expect(problems.join(' ')).toContain('App.page');
    });

    it('트레이 팝업은 그 창 slot을 선언한 것만, 메인은 메인 slot이나 선언 없는 것을 읽는다', () => {
        const tray = { gui: gui({ 'TrayPopup.section': {} }) as never };
        const main = { gui: gui({ 'App.page': {} }) as never };
        const none = { gui: gui({}) as never };
        expect(loadsOn(tray, 'tray')).toBe(true);
        expect(loadsOn(tray, 'main')).toBe(false);
        expect(loadsOn(main, 'tray')).toBe(false);
        expect(loadsOn(main, 'main')).toBe(true);
        expect(loadsOn(none, 'main')).toBe(true);
        expect(loadsOn(none, 'tray')).toBe(false);
    });
});

describe('번들 읽기와 slot 등록', () => {
    beforeEach(() => {
        window.api.extensionGuiBundle = vi.fn().mockResolvedValue(null);
        window.api.extensionGuiStyles = vi.fn().mockResolvedValue(null);
        window.api.extensionI18n = vi.fn().mockResolvedValue(null);
        vi.spyOn(console, 'warn').mockImplementation(() => {});
    });
    afterEach(() => {
        for (const k of ['extensionList', 'extensionGuiBundle', 'extensionGuiStyles', 'extensionI18n'])
            delete (window.api as unknown as Record<string, unknown>)[k];
        delete window.SabaExtPage;
        delete window.SabaExtTray;
        vi.restoreAllMocks();
    });

    it('등록한 컴포넌트는 넣은 익스텐션과 manifest 표시 정보를 함께 가진다', async () => {
        const Page = () => null;
        window.SabaExtPage = { registerSlots: () => ({ 'App.page': Page }) };
        window.api.extensionList = vi
            .fn()
            .mockResolvedValue({ extensions: [ext('page', { 'App.page': { label: 'title', icon: 'globe' } })] });
        const { result } = renderHook(() => useExtensions(), { wrapper: ExtensionProvider });
        await waitFor(() => expect(result.current.slots['App.page']).toHaveLength(1));
        expect(result.current.slots['App.page'][0]).toEqual({ ext: 'page', Component: Page, meta: { label: 'title', icon: 'globe' } });
    });

    it('번들 없이 설정 탭만 선언하면 자동 폼 칸이 생긴다', async () => {
        window.api.extensionList = vi.fn().mockResolvedValue({
            extensions: [ext('cfg', { 'Settings.tab': { label: 'Cfg' } }, { gui: gui({ 'Settings.tab': { label: 'Cfg' } }, null) })],
        });
        const { result } = renderHook(() => useExtensions(), { wrapper: ExtensionProvider });
        await waitFor(() => expect(result.current.slots['Settings.tab']).toHaveLength(1));
        expect(result.current.slots['Settings.tab'][0].Component).toBeUndefined();
        expect(window.api.extensionGuiBundle).not.toHaveBeenCalled();
    });

    it('트레이 창은 트레이 slot을 선언한 번들만 읽고 허락을 묻지 않는다', async () => {
        const Section = () => null;
        window.SabaExtTray = { registerSlots: () => ({ 'TrayPopup.section': Section }) };
        window.api.extensionList = vi.fn().mockResolvedValue({
            extensions: [
                ext('tray', { 'TrayPopup.section': {} }),
                ext('page', { 'App.page': {} }),
                ext('ask', { 'TrayPopup.section': {} }, { gui_allowed: false }),
                { id: 'plain', name: 'plain', enabled: true, version: '1.0.0', gui: null },
            ],
        });
        const wrapper = ({ children }: { children: React.ReactNode }) => (
            <ExtensionProvider surface="tray">{children}</ExtensionProvider>
        );
        const { result } = renderHook(() => useExtensions(), { wrapper });
        await waitFor(() => expect(result.current.slots['TrayPopup.section']).toHaveLength(1));
        expect(result.current.slots['App.page']).toBeUndefined();
        expect(result.current.guiPending).toEqual([]);
        // 트레이 slot이 없는 익스텐션의 번역도 읽지 않는다 (열 때마다 새로 뜨는 창)
        expect(window.api.extensionI18n).not.toHaveBeenCalledWith('page', expect.anything());
        expect(window.api.extensionI18n).not.toHaveBeenCalledWith('plain', expect.anything());
    });
});

describe('host 통로', () => {
    it('openApp은 메인 창의 정해진 화면, ext:<id>, server:<인스턴스 id>만 받는다', () => {
        for (const ok of ['servers', 'discord', 'activity', 'settings', 'ext:pet', 'ext:my_ext-2', 'server:inst-1'])
            expect(isAppRoute(ok)).toBe(true);
        for (const bad of [undefined, '', 'nowhere', 'ext:', 'ext:a b', 'ext:a.b', 'server:', 'server:a/b', 42])
            expect(isAppRoute(bad)).toBe(false);
    });

    it('slot 컴포넌트는 자기 익스텐션 id가 붙은 host와 창의 동작을 받는다', async () => {
        let got: ExtensionHost | null = null;
        const Comp = ({ host }: { host: ExtensionHost }) => {
            got = host;
            return <span>in slot</span>;
        };
        const run = vi.fn().mockResolvedValue(true);
        const win = { servers: () => [], run, openApp: vi.fn() };
        render(
            <HostWindowContext.Provider value={win}>
                <SlotView entry={{ ext: 'demo', Component: Comp, meta: {} }} />
            </HostWindowContext.Provider>,
        );
        expect(await screen.findByText('in slot')).toBeTruthy();
        expect(got!.extension).toBe('demo');
        expect(got!.version).toBe(1);
        await got!.run('srv-1', 'stop');
        expect(run).toHaveBeenCalledWith('srv-1', 'stop');
    });

    it('host.on은 고른 종류의 데몬 이벤트만 넘기고 끊을 수 있다', () => {
        let emit: (raw: unknown) => void = () => {};
        const off = vi.fn();
        window.api.onDaemonEvent = vi.fn((cb: (raw: unknown) => void) => {
            emit = cb;
            return off;
        }) as never;
        let got: ExtensionHost | null = null;
        const Comp = ({ host }: { host: ExtensionHost }) => {
            got = host;
            return null;
        };
        render(<SlotView entry={{ ext: 'demo', Component: Comp, meta: {} }} />);
        const seen: string[] = [];
        const stop = got!.on('instance.started', (data) => seen.push(String(data.instance)));
        emit({ type: 'instance.stopped', data: { instance: 'A' } });
        emit({ type: 'instance.started', data: { instance: 'B' } });
        stop();
        expect(seen).toEqual(['B']);
        expect(off).toHaveBeenCalled();
        delete (window.api as unknown as Record<string, unknown>).onDaemonEvent;
    });

    it('기존 inline slot도 host를 함께 받는다', async () => {
        const Comp = ({ host, server }: { host: ExtensionHost; server: { id: string } }) => (
            <span>{`${host.extension}:${server.id}`}</span>
        );
        window.SabaExtPage = { registerSlots: () => ({ 'ServerCard.badge': Comp }) };
        window.api.extensionList = vi.fn().mockResolvedValue({ extensions: [ext('page', { 'ServerCard.badge': {} })] });
        window.api.extensionGuiBundle = vi.fn().mockResolvedValue(null);
        window.api.extensionGuiStyles = vi.fn().mockResolvedValue(null);
        window.api.extensionI18n = vi.fn().mockResolvedValue(null);
        render(
            <ExtensionProvider>
                <ExtensionSlot slotId="ServerCard.badge" server={{ id: 's1' }} />
            </ExtensionProvider>,
        );
        expect(await screen.findByText('page:s1')).toBeTruthy();
        delete window.SabaExtPage;
    });
});

describe('탭형 slot', () => {
    beforeEach(() => {
        window.api.extensionGuiBundle = vi.fn().mockResolvedValue(null);
        window.api.extensionGuiStyles = vi.fn().mockResolvedValue(null);
        window.api.extensionI18n = vi.fn().mockResolvedValue(null);
        vi.spyOn(console, 'warn').mockImplementation(() => {});
    });
    afterEach(() => {
        delete window.SabaExtPage;
        vi.restoreAllMocks();
    });

    it('익스텐션마다 한 칸, id는 ext:<id>, 이름은 manifest label → 익스텐션 이름 순', async () => {
        const A = () => null;
        const B = () => null;
        window.SabaExtPage = { registerSlots: () => ({ 'App.page': [A, B] }) };
        window.api.extensionList = vi.fn().mockResolvedValue({
            extensions: [ext('page', { 'App.page': { icon: 'globe' } })],
        });
        const { result } = renderHook(() => useSlotPages('App.page'), { wrapper: ExtensionProvider });
        await waitFor(() => expect(result.current).toHaveLength(1));
        expect(result.current[0]).toMatchObject({ id: 'ext:page', ext: 'page', label: 'page name', icon: 'globe' });
        expect(result.current[0].entry.Component).toBe(A);
    });
});
