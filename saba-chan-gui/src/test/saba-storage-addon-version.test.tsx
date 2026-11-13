/**
 * 사바 스토리지의 모듈·익스텐션 카드에서 버전을 골라 설치하고(고정), 자동 업데이트로 돌아간다.
 * 고를 수 있는 버전·서명 확인·설치·기다림은 데몬이 정한다. 화면은 데몬 답을 보여 준다.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import i18n from '../i18n';

const ext = vi.hoisted(() => ({ extensions: [] as Record<string, unknown>[], refreshExtensions: vi.fn() }));
vi.mock('../contexts/ExtensionContext', () => ({
    useExtensions: () => ({ ...ext, loading: false, toggleExtension: vi.fn(), removeExtension: vi.fn() }),
}));
// 카드 안 감사 알림은 이 시험과 상관없다
vi.mock('../components/servers/ModuleAuditNotice', () => ({
    clearModuleAuditCache: vi.fn(),
    ExtensionAuditNotice: () => null,
}));

import SabaStorage from '../components/Modals/SabaStorage';
import { ContextMenuProvider } from '../components/ui/ContextMenu';

const tr = (key: string, vars: Record<string, unknown> = {}) => i18n.t(`gui:saba_storage.${key}`, vars);
const kind = () => tr('version_kind_module');

const MINECRAFT = { name: '마인크래프트', version: '0.1.13', path: '/m/minecraft', description: '서버를 실행해요', trust: 'verified' };
const VERSIONS = {
    ok: true,
    key: 'module-minecraft',
    core: '0.1.14',
    installed: '0.1.13',
    versions: [
        { version: '0.1.13', installed: true, latest: true, yanked: false },
        { version: '0.1.12', installed: false, latest: false, yanked: false },
        { version: '0.1.11', installed: false, latest: false, yanked: true },
        { version: '0.1.10', installed: false, latest: false, yanked: false },
    ],
    excluded: ['0.1.14'],
    pinned: null,
};

function fakeApi(overrides: Record<string, unknown>) {
    window.api = new Proxy<Record<string | symbol, unknown>>(
        {
            moduleList: vi.fn().mockResolvedValue({ modules: [MINECRAFT] }),
            moduleManifest: vi.fn().mockResolvedValue({ ok: true, manifest: { modules: {} } }),
            addonVersions: vi.fn().mockResolvedValue(VERSIONS),
            ...overrides,
        },
        {
            get(target, prop) {
                if (typeof prop === 'symbol') return target[prop];
                if (!(prop in target)) target[prop] = vi.fn().mockResolvedValue({});
                return target[prop];
            },
        },
    ) as unknown as typeof window.api;
}

beforeEach(() => {
    window.showToast = vi.fn();
    ext.extensions = [];
});

afterEach(() => {
    delete (window as { api?: unknown }).api;
});

const pane = () => document.querySelector('.saba-storage-pane:not([hidden])')!;

async function openTab(index: number) {
    render(
        <ContextMenuProvider allowNative={false}>
            <SabaStorage />
        </ContextMenuProvider>,
    );
    await act(async () => fireEvent.click(document.querySelectorAll('.saba-storage-tab')[index]));
    await waitFor(() => expect(pane().querySelector('.ss-cards .ss-card')).not.toBeNull());
}

async function chooseFromMenu(label: string) {
    await act(async () => fireEvent.contextMenu(pane().querySelector('.ss-cards .ss-card')!));
    await act(async () => fireEvent.click(screen.getByRole('menuitem', { name: label })));
}

/** 버전 선택 창에서 0.1.12를 골라 설치까지 누른다 */
async function pick012() {
    await chooseFromMenu(tr('version_choose'));
    const row = await screen.findByRole('radio', { name: /0\.1\.12/ });
    await act(async () => fireEvent.click(row));
    await act(async () => fireEvent.click(screen.getByRole('button', { name: tr('version_install') })));
    await act(async () => fireEvent.click(screen.getByRole('button', { name: tr('version_confirm_go') })));
}

describe('모듈 버전 선택', () => {
    it('창은 데몬이 준 버전을 보여 주고, 내려진 버전과 설치된 버전은 고를 수 없으며 뺀 버전 수를 알린다', async () => {
        fakeApi({});
        await openTab(1);
        await chooseFromMenu(tr('version_choose'));
        expect(window.api.addonVersions).toHaveBeenCalledWith('module-minecraft');
        await screen.findByText(tr('version_title', { name: '마인크래프트', kind: kind() }));
        expect(screen.getByText(tr('version_subtitle', { core: '0.1.14' }))).toBeTruthy();
        expect(screen.getByRole('radio', { name: /0\.1\.11/ })).toHaveProperty('disabled', true);
        expect(screen.getByRole('radio', { name: /0\.1\.13/ })).toHaveProperty('disabled', true);
        expect(screen.getByText(tr('version_yanked_hint'))).toBeTruthy();
        expect(screen.getByText(tr('version_excluded', { count: 1, versions: '0.1.14' }))).toBeTruthy();
        // 고르기 전에는 설치할 수 없다
        expect(screen.getByRole('button', { name: tr('version_install') })).toHaveProperty('disabled', true);
    });

    it('고른 버전을 확인받고 데몬에 보내, 바꿨다고 알린다', async () => {
        fakeApi({ addonPin: vi.fn().mockResolvedValue({ ok: true, state: 'applied', version: '0.1.12', waiting_for: [] }) });
        await openTab(1);
        await chooseFromMenu(tr('version_choose'));
        await act(async () => fireEvent.click(await screen.findByRole('radio', { name: /0\.1\.12/ })));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: tr('version_install') })));
        expect(screen.getByText(tr('version_confirm_title', { version: '0.1.12' }))).toBeTruthy();
        expect(
            screen.getByText(
                tr('version_confirm_message', { name: '마인크래프트', kind: kind(), from: '0.1.13', version: '0.1.12', until: '0.1.13' }),
            ),
        ).toBeTruthy();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: tr('version_confirm_go') })));
        expect(window.api.addonPin).toHaveBeenCalledWith('module-minecraft', '0.1.12');
        await screen.findByText(tr('version_done', { name: '마인크래프트', kind: kind(), version: '0.1.12' }));
    });

    it('켜진 서버를 기다리면 카드에 어느 서버가 꺼지면 바꾸는지 보여 준다', async () => {
        const moduleList = vi
            .fn()
            .mockResolvedValueOnce({ modules: [MINECRAFT] })
            .mockResolvedValue({ modules: [{ ...MINECRAFT, pending_version: '0.1.12' }] });
        fakeApi({
            moduleList,
            addonPin: vi.fn().mockResolvedValue({ ok: true, state: 'waiting', version: '0.1.12', waiting_for: ['친구들 서버'] }),
        });
        await openTab(1);
        await pick012();
        await screen.findByText(tr('version_waiting', { servers: '친구들 서버', version: '0.1.12' }));
    });

    it('바꾸지 못하면 이유와 그대로인 버전을 알린다', async () => {
        fakeApi({
            addonPin: vi.fn().mockResolvedValue({ ok: false, error: 'download failed', error_code: 'download_failed' }),
        });
        await openTab(1);
        await pick012();
        await screen.findByText(
            tr('version_failed', {
                name: '마인크래프트',
                kind: kind(),
                version: '0.1.12',
                reason: tr('version_error.download_failed'),
                from: '0.1.13',
            }),
        );
    });

    it('고정된 카드는 고정 표시와 설명을 보이고, 자동 업데이트로 돌아가기를 확인받아 데몬에 보낸다', async () => {
        const pinned = { ...MINECRAFT, version: '0.1.12', pinned: { version: '0.1.12', until_newer_than: '0.1.13' } };
        fakeApi({
            moduleList: vi.fn().mockResolvedValue({ modules: [pinned] }),
            addonUnpin: vi.fn().mockResolvedValue({ ok: true, state: 'applied', version: '0.1.13', waiting_for: [] }),
        });
        await openTab(1);
        expect(pane().textContent).toContain(tr('version_pinned'));
        expect(pane().textContent).toContain(tr('version_pinned_note', { until: '0.1.13' }));
        await chooseFromMenu(tr('version_unpin'));
        expect(screen.getByText(tr('version_unpin_message', { name: '마인크래프트', kind: kind(), version: '0.1.13' }))).toBeTruthy();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: tr('version_unpin_go') })));
        expect(window.api.addonUnpin).toHaveBeenCalledWith('module-minecraft');
        await screen.findByText(tr('version_done', { name: '마인크래프트', kind: kind(), version: '0.1.13' }));
    });

    it('고정되지 않은 카드에는 자동 업데이트로 돌아가기가 없다', async () => {
        fakeApi({});
        await openTab(1);
        await act(async () => fireEvent.contextMenu(pane().querySelector('.ss-cards .ss-card')!));
        expect(screen.getByRole('menuitem', { name: tr('version_choose') })).toBeTruthy();
        expect(screen.queryByRole('menuitem', { name: tr('version_unpin') })).toBeNull();
    });
});

describe('익스텐션 버전 선택', () => {
    it('익스텐션 카드 메뉴도 버전 선택을 열고 ext- 키로 데몬에 묻는다', async () => {
        ext.extensions = [{ id: 'steamcmd', name: 'SteamCMD', version: '0.1.8', enabled: true }];
        fakeApi({ addonVersions: vi.fn().mockResolvedValue({ ...VERSIONS, key: 'ext-steamcmd' }) });
        await openTab(2);
        await chooseFromMenu(tr('version_choose'));
        expect(window.api.addonVersions).toHaveBeenCalledWith('ext-steamcmd');
        await screen.findByText(tr('version_title', { name: 'SteamCMD', kind: tr('version_kind_ext') }));
    });
});

describe('버전 선택 브리지', () => {
    const setup = () => {
        const invoke = vi.fn().mockResolvedValue({ status: 200, data: { ok: true } });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } as never });
        return { api, sent: () => invoke.mock.calls.at(-1)![1] };
    };

    it('버전 목록·고정·고정 풀기를 데몬 라우트로 부른다', async () => {
        const { api, sent } = setup();
        await api.addonVersions('module-minecraft');
        expect(sent()).toMatchObject({ method: 'GET', path: '/api/updates/addons/module-minecraft/versions' });
        await api.addonPin('ext-steamcmd', '0.1.8');
        expect(sent()).toMatchObject({ method: 'POST', path: '/api/updates/addons/ext-steamcmd/pin', body: { version: '0.1.8' } });
        await api.addonUnpin('ext-steamcmd');
        expect(sent()).toMatchObject({ method: 'DELETE', path: '/api/updates/addons/ext-steamcmd/pin' });
    });
});
