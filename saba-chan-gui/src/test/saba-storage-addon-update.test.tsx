/**
 * 사바 스토리지 모듈·익스텐션 카드의 업데이트 단추.
 * 새 버전이 있는지는 데몬의 업데이트 상태가 정하고, 바꾸기(받기·검증·켜진 서버 기다리기)는 데몬의 "최신으로 돌아가기"가 한다.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import i18n from '../i18n';

const ext = vi.hoisted(() => ({ extensions: [] as Record<string, unknown>[], refreshExtensions: vi.fn() }));
vi.mock('../contexts/ExtensionContext', () => ({
    useExtensions: () => ({ ...ext, loading: false, toggleExtension: vi.fn(), removeExtension: vi.fn() }),
}));
vi.mock('../components/servers/ModuleAuditNotice', () => ({
    clearModuleAuditCache: vi.fn(),
    ExtensionAuditNotice: () => null,
}));

import SabaStorage from '../components/Modals/SabaStorage';
import { ContextMenuProvider } from '../components/ui/ContextMenu';

const tr = (key: string, vars: Record<string, unknown> = {}) => i18n.t(`gui:saba_storage.${key}`, vars);

const MINECRAFT = { name: '마인크래프트', version: '0.1.13', path: '/m/minecraft', trust: 'verified' };
const PALWORLD = { name: '팰월드', version: '0.1.13', path: '/m/palworld', trust: 'verified' };

const newer = (component: string, latest: string, kind = 'module') => ({
    component,
    kind,
    update_available: true,
    current_version: '0.1.13',
    latest_version: latest,
});

function fakeApi(modules: Record<string, unknown>[], components: Record<string, unknown>[], overrides: Record<string, unknown> = {}) {
    window.api = new Proxy<Record<string | symbol, unknown>>(
        {
            moduleList: vi.fn().mockResolvedValue({ modules }),
            moduleManifest: vi.fn().mockResolvedValue({ success: true, manifest: { modules: {} } }),
            updaterStatus: vi.fn().mockResolvedValue({ success: true, components }),
            addonUnpin: vi.fn().mockResolvedValue({ success: true, state: 'applied', version: '0.1.14', waiting_for: [] }),
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
const card = (name: string) => [...pane().querySelectorAll('.ss-cards .ss-card')].find((c) => c.textContent?.includes(name))!;
const updateButton = (name: string) =>
    [...card(name).querySelectorAll('button')].find((b) => b.textContent === tr('update')) || null;

async function openTab(index: number) {
    render(
        <ContextMenuProvider allowNative={false}>
            <SabaStorage />
        </ContextMenuProvider>,
    );
    await act(async () => fireEvent.click(document.querySelectorAll('.saba-storage-tab')[index]));
    await waitFor(() => expect(pane().querySelector('.ss-cards .ss-card')).not.toBeNull());
}

describe('모듈 업데이트 단추', () => {
    it('새 버전이 있는 카드에만 새 버전과 업데이트 단추가 보인다', async () => {
        fakeApi([MINECRAFT, PALWORLD], [newer('module-minecraft', '0.1.14'), { component: 'module-palworld', kind: 'module', update_available: false }]);
        await openTab(1);
        await waitFor(() => expect(updateButton('마인크래프트')).not.toBeNull());
        expect(card('마인크래프트').querySelector('.ss-update-badge')).toHaveTextContent('v0.1.14');
        expect(updateButton('팰월드')).toBeNull();
        expect(card('팰월드').querySelector('.ss-update-badge')).toBeNull();
    });

    it('누르면 데몬에 최신으로 바꾸라고 보내고 바꿨다고 알린다', async () => {
        fakeApi([MINECRAFT], [newer('module-minecraft', '0.1.14')]);
        await openTab(1);
        await waitFor(() => expect(updateButton('마인크래프트')).not.toBeNull());
        await act(async () => fireEvent.click(updateButton('마인크래프트')!));
        expect(window.api.addonUnpin).toHaveBeenCalledWith('module-minecraft');
        expect(
            await screen.findByText(tr('version_done', { name: '마인크래프트', kind: tr('version_kind_module'), version: '0.1.14' })),
        ).toBeTruthy();
    });

    it('버전을 고정한 카드에는 업데이트 단추가 없다', async () => {
        const pinned = { ...MINECRAFT, pinned: { version: '0.1.13', until_newer_than: '0.1.14' } };
        fakeApi([pinned], [newer('module-minecraft', '0.1.14')]);
        await openTab(1);
        await waitFor(() => expect(window.api.updaterStatus).toHaveBeenCalled());
        await waitFor(() => expect(card('마인크래프트')).toBeTruthy());
        expect(updateButton('마인크래프트')).toBeNull();
    });

    it('새 버전이 둘 이상이면 모두 업데이트가 보이고 하나씩 차례로 바꾼다', async () => {
        fakeApi([MINECRAFT, PALWORLD], [newer('module-minecraft', '0.1.14'), newer('module-palworld', '0.1.15')]);
        await openTab(1);
        const all = await screen.findByRole('button', { name: new RegExp(tr('update_all')) });
        await act(async () => fireEvent.click(all));
        await waitFor(() => expect(window.api.addonUnpin).toHaveBeenCalledTimes(2));
        expect(vi.mocked(window.api.addonUnpin).mock.calls.map((c) => c[0]).sort()).toEqual(['module-minecraft', 'module-palworld']);
    });

    it('모두 업데이트 중 하나가 실패하면 뒤 카드가 성공해도 실패 알림이 남는다', async () => {
        const addonUnpin = vi.fn(async (key: string) =>
            key === 'module-minecraft'
                ? { success: false, error_code: 'module_modified', error: 'modified' }
                : { success: true, state: 'applied', version: '0.1.15', waiting_for: [] },
        );
        fakeApi([MINECRAFT, PALWORLD], [newer('module-minecraft', '0.1.14'), newer('module-palworld', '0.1.15')], { addonUnpin });
        await openTab(1);
        const all = await screen.findByRole('button', { name: new RegExp(tr('update_all')) });
        await act(async () => fireEvent.click(all));
        await waitFor(() => expect(addonUnpin).toHaveBeenCalledTimes(2));
        await waitFor(() => expect(document.querySelector('.update-modal-status-bar')).not.toBeNull());
        expect(document.querySelector('.update-modal-status-bar.error')).toHaveTextContent(tr('version_error.modified'));
    });

    it('새 버전이 하나면 모두 업데이트는 없다', async () => {
        fakeApi([MINECRAFT, PALWORLD], [newer('module-minecraft', '0.1.14')]);
        await openTab(1);
        await waitFor(() => expect(updateButton('마인크래프트')).not.toBeNull());
        expect(screen.queryByRole('button', { name: new RegExp(tr('update_all')) })).toBeNull();
    });
});

describe('익스텐션 업데이트 단추', () => {
    it('익스텐션 카드도 데몬 상태의 새 버전을 보이고 ext- 키로 바꾼다', async () => {
        ext.extensions = [{ id: 'steamcmd', name: 'SteamCMD', version: '0.1.9', enabled: true }];
        fakeApi([], [newer('ext-steamcmd', '0.1.10', 'extension')]);
        await openTab(2);
        await waitFor(() => expect(updateButton('SteamCMD')).not.toBeNull());
        expect(card('SteamCMD').querySelector('.ss-update-badge')).toHaveTextContent('v0.1.10');
        await act(async () => fireEvent.click(updateButton('SteamCMD')!));
        expect(window.api.addonUnpin).toHaveBeenCalledWith('ext-steamcmd');
    });
});
