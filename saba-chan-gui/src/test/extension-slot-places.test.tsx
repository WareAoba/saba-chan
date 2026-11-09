/**
 * 새 slot 자리 (익스텐션 slot 확장 1단계, 사용자 2026-11-09): 맨 위 탭·아이콘 버튼·설정 탭·서버 추가 카드·트레이 칸.
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SlotEntry } from '../contexts/ExtensionContext';
import type { ExtensionHost } from '../extensions/host';

let slots: Record<string, SlotEntry[]> = {};
let extensions: Record<string, unknown>[] = [];
vi.mock('../contexts/ExtensionContext', async (orig) => ({
    ...(await orig<typeof import('../contexts/ExtensionContext')>()),
    useExtensions: () => ({ slots, extensions, loading: false }),
}));

const { APP_PAGE_INLINE, ExtensionAppBarActions, ExtensionPageTabs } = await import('../components/shell/ExtensionAppBar');
const { ExtensionSettingsPanel } = await import('../components/Modals/ExtensionSettingsPanel');
const { AddInstanceModal } = await import('../components/Modals/AddInstanceModal');
const { TrayPopup } = await import('../components/shell/TrayPopup');

const entry = (ext: string, Component?: SlotEntry['Component'], meta = {}): SlotEntry => ({ ext, Component, meta });
const flush = () => act(async () => {});

afterEach(() => {
    slots = {};
    extensions = [];
});

describe('맨 위 탭 줄 (App.page)', () => {
    it('익스텐션 화면을 탭으로 보이고, 넘치면 더보기 하나로 묶는다', () => {
        const ids = Array.from({ length: APP_PAGE_INLINE + 2 }, (_, i) => `p${i}`);
        slots = { 'App.page': ids.map((id) => entry(id, () => null, { label: `${id} tab` })) };
        const onNavigate = vi.fn();
        render(<ExtensionPageTabs page="servers" onNavigate={onNavigate} />);
        fireEvent.click(screen.getByText('p0 tab'));
        expect(onNavigate).toHaveBeenCalledWith('ext:p0');
        expect(screen.queryByText(`p${APP_PAGE_INLINE} tab`)).toBeNull();
        expect(screen.getAllByRole('button')).toHaveLength(APP_PAGE_INLINE + 1);
    });

    it('넘친 화면을 보고 있으면 더보기 자리에 그 이름을 보인다', () => {
        const ids = Array.from({ length: APP_PAGE_INLINE + 1 }, (_, i) => `p${i}`);
        slots = { 'App.page': ids.map((id) => entry(id, () => null, { label: `${id} tab` })) };
        render(<ExtensionPageTabs page={`ext:p${APP_PAGE_INLINE}`} onNavigate={() => {}} />);
        expect(screen.getByText(`p${APP_PAGE_INLINE} tab`).closest('button')?.getAttribute('aria-current')).toBe('page');
    });
});

describe('오른쪽 위 아이콘 버튼 (AppBar.actions)', () => {
    it('누르면 작은 창에 컴포넌트를 그리고 close로 닫는다', () => {
        const Panel = ({ close, host }: { close: () => void; host: ExtensionHost }) => (
            <button type="button" onClick={close}>
                {`panel of ${host.extension}`}
            </button>
        );
        slots = { 'AppBar.actions': [entry('quick', Panel, { label: 'Quick' })] };
        render(<ExtensionAppBarActions />);
        fireEvent.click(screen.getByLabelText('Quick'));
        fireEvent.click(screen.getByText('panel of quick'));
        expect(screen.queryByText('panel of quick')).toBeNull();
    });
});

describe('설정 탭 (Settings.tab)', () => {
    beforeEach(() => {
        window.api.extensionConfigGet = vi.fn().mockResolvedValue({ config: { volume: 7 } });
        window.api.extensionConfigSet = vi.fn().mockResolvedValue({ ok: true });
    });
    afterEach(() => {
        delete (window.api as unknown as Record<string, unknown>).extensionConfigGet;
        delete (window.api as unknown as Record<string, unknown>).extensionConfigSet;
    });

    it('컴포넌트가 없으면 config_fields 폼을 펼친 채로 보인다', async () => {
        extensions = [{ id: 'cfg', name: 'Cfg', config_fields: { volume: { type: 'number' } } }];
        render(<ExtensionSettingsPanel entry={entry('cfg')} />);
        await flush();
        expect((screen.getByLabelText('volume') as HTMLInputElement).value).toBe('7');
    });

    it('컴포넌트는 같은 설정을 getConfig·setConfig로 읽고 쓴다', async () => {
        let api: { getConfig: () => Promise<unknown>; setConfig: (c: object) => Promise<unknown> } | null = null;
        const Comp = (p: typeof api) => {
            api = p;
            return null;
        };
        extensions = [{ id: 'cfg', name: 'Cfg', config_fields: {} }];
        render(<ExtensionSettingsPanel entry={entry('cfg', Comp as never)} />);
        await api!.getConfig();
        await api!.setConfig({ volume: 3 });
        expect(window.api.extensionConfigGet).toHaveBeenCalledWith('cfg');
        expect(window.api.extensionConfigSet).toHaveBeenCalledWith('cfg', { volume: 3 });
    });
});

describe('서버 추가 카드 (AddServer.method)', () => {
    it('세 번째 카드를 누르면 그 화면을 그리고, onBack으로 고르기 화면에 돌아온다', async () => {
        const Flow = ({ onBack }: { onBack: () => void }) => (
            <button type="button" onClick={onBack}>
                modpack flow
            </button>
        );
        slots = { 'AddServer.method': [entry('modpack', Flow, { label: 'From modpack' })] };
        render(<AddInstanceModal isOpen onClose={() => {}} extensions={[]} servers={[]} onAddServer={async () => null} />);
        fireEvent.click(screen.getByText('From modpack'));
        fireEvent.click(await screen.findByText('modpack flow'));
        expect(await screen.findByText('From modpack')).toBeTruthy();
    });

    it('그 화면을 보는 사이 익스텐션이 빠지면 빈 창이 아니라 고르기 화면으로 돌아온다', async () => {
        const Flow = () => <span>modpack flow</span>;
        slots = { 'AddServer.method': [entry('modpack', Flow, { label: 'From modpack' })] };
        const props = { isOpen: true, onClose: () => {}, extensions: [], servers: [], onAddServer: async () => null };
        const { rerender } = render(<AddInstanceModal {...props} />);
        fireEvent.click(screen.getByText('From modpack'));
        expect(await screen.findByText('modpack flow')).toBeTruthy();
        slots = {};
        rerender(<AddInstanceModal {...props} />);
        await flush();
        expect(screen.queryByText('modpack flow')).toBeNull();
        expect(document.querySelector('.ai-choice-card')).not.toBeNull();
    });
});

describe('트레이 칸 (TrayPopup.section)', () => {
    beforeEach(() => {
        window.api.daemonStatus = vi.fn().mockResolvedValue({ running: true });
        window.api.serverList = vi.fn().mockResolvedValue({ servers: [{ id: 'a', name: 'A', status: 'running' }] });
        window.api.instanceStop = vi.fn().mockResolvedValue({ success: true });
        window.api.trayPopupFit = vi.fn();
        window.api.daemonTarget = vi.fn().mockResolvedValue({ target: null });
    });
    afterEach(() => {
        for (const k of ['daemonStatus', 'serverList', 'instanceStop', 'trayPopupFit', 'daemonTarget', 'remoteDaemons'])
            delete (window.api as unknown as Record<string, unknown>)[k];
    });

    it('서버 목록을 받고, 이 PC 데몬이면 host.run으로 정지할 수 있다', async () => {
        let host: ExtensionHost | null = null;
        const Section = (p: { host: ExtensionHost; servers: { name: string }[] }) => {
            host = p.host;
            return <span>{`servers: ${p.servers.map((s) => s.name).join(',')}`}</span>;
        };
        slots = { 'TrayPopup.section': [entry('mini', Section)] };
        render(<TrayPopup />);
        await flush();
        expect(screen.getByText('servers: A')).toBeTruthy();
        expect(await host!.run('a', 'stop')).toBe(true);
        expect(window.api.instanceStop).toHaveBeenCalledWith('a');
    });

    it('원격 데몬 서버의 정지는 익스텐션이 하지 못한다 (이 창에서 두 번 눌러 확정하는 규칙)', async () => {
        window.api.daemonTarget = vi.fn().mockResolvedValue({ target: 'host-b' });
        window.api.remoteDaemons = vi.fn().mockResolvedValue({ daemons: [{ hostId: 'host-b', name: 'B' }] });
        let host: ExtensionHost | null = null;
        const Section = (p: { host: ExtensionHost }) => {
            host = p.host;
            return null;
        };
        slots = { 'TrayPopup.section': [entry('mini', Section)] };
        render(<TrayPopup />);
        await flush();
        expect(await host!.run('a', 'stop')).toBe(false);
        expect(window.api.instanceStop).not.toHaveBeenCalled();
    });

    it('원격 데몬인지 아직 모르는 동안에도 정지·재시작은 하지 못한다', async () => {
        window.api.daemonTarget = vi.fn().mockResolvedValue({ target: 'host-b' });
        window.api.remoteDaemons = vi.fn().mockResolvedValue({ daemons: [{ hostId: 'host-b', name: 'B' }] });
        window.api.serverList = vi.fn(() => new Promise<never>(() => {}));
        let host: ExtensionHost | null = null;
        const Section = (p: { host: ExtensionHost }) => {
            host = p.host;
            return null;
        };
        slots = { 'TrayPopup.section': [entry('mini', Section)] };
        render(<TrayPopup />);
        await flush();
        expect(host).not.toBeNull();
        expect(await host!.run('a', 'stop')).toBe(false);
        expect(window.api.instanceStop).not.toHaveBeenCalled();
    });
});
