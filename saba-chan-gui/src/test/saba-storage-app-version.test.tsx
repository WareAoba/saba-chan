/**
 * 사바 스토리지 컴포넌트 탭의 "사바쨩 버전" 줄: 이 PC에 남겨 둔 직전 버전으로 되돌리거나 자동 업데이트로 돌아간다.
 * 고를 수 있는 버전·되돌리기·업데이터 띄우기는 데몬이 정한다. 화면은 데몬 답을 보여 주고 고른 것을 보낸다.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import i18n from '../i18n';

vi.mock('../contexts/ExtensionContext', () => ({
    useExtensions: () => ({ extensions: [], loading: false, refreshExtensions: vi.fn() }),
}));

import { announceLastUpdateResult } from '../components/Modals/PatchNotesModal';
import SabaStorage from '../components/Modals/SabaStorage';
import { useUIStore } from '../stores/useUIStore';

const tr = (key: string, vars: Record<string, unknown> = {}) => i18n.t(`gui:${key}`, vars);
const ss = (key: string, vars: Record<string, unknown> = {}) => tr(`saba_storage.${key}`, vars);

const UPDATED_AT = '2026-11-12T03:09:00Z';
const KEPT = { version: '0.1.13', updated_at: UPDATED_AT, needs_newer_core: [] as string[] };
const VERSIONS = {
    ok: true,
    channel: 'stable',
    installed: '0.1.14',
    kept_previous: KEPT,
    latest: null,
    held_until_newer_than: null,
};
const HELD = {
    ok: true,
    channel: 'stable',
    installed: '0.1.13',
    kept_previous: null,
    latest: '0.1.14',
    held_until_newer_than: '0.1.14',
};
const STATUS = {
    ok: true,
    components: [
        { component: 'saba-core', display_name: 'Saba-Core', current_version: '0.1.13' },
        { component: 'gui', display_name: 'GUI', current_version: '0.1.13' },
        { component: 'module-minecraft', display_name: '마인크래프트 모듈', current_version: '1.2.0' },
    ],
};

function fakeApi(overrides: Record<string, unknown>) {
    window.api = new Proxy<Record<string | symbol, unknown>>(
        {
            updaterGetConfig: vi.fn().mockResolvedValue({ config: { enabled: true, channel: 'stable' } }),
            updaterStatus: vi.fn().mockResolvedValue(STATUS),
            updaterDownloadProgress: vi.fn().mockResolvedValue({ ok: true, active: false }),
            daemonTarget: vi.fn().mockResolvedValue({ target: null }),
            updaterVersions: vi.fn().mockResolvedValue(VERSIONS),
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

const settings = () => document.querySelector('.update-panel-settings')!;
const row = () => document.querySelector('.app-version-row');

async function openTab() {
    render(<SabaStorage />);
    await waitFor(() => expect(window.api.updaterVersions).toHaveBeenCalled());
}

async function openDialog() {
    await openTab();
    await act(async () => fireEvent.click(await screen.findByRole('button', { name: ss('app_version_choose') })));
    await screen.findByText(ss('app_version_title'));
}

const when = new Date(UPDATED_AT).toLocaleString(i18n.language, {
    month: 'long',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
});

beforeEach(() => {
    window.showToast = vi.fn();
});

afterEach(() => {
    delete (window as { api?: unknown }).api;
    useUIStore.setState({ revertResult: null });
});

describe('사바쨩 버전 줄', () => {
    it('직전 버전을 남겨 두었으면 지금 버전과 함께 알리고, 다른 버전 선택을 보인다', async () => {
        fakeApi({});
        await openTab();
        await waitFor(() =>
            expect(settings().textContent).toContain(
                ss('app_version_now_kept', { current: '0.1.14', previous: '0.1.13' }),
            ),
        );
        expect(settings().textContent).toContain(ss('app_version_label'));
        expect(screen.getByRole('button', { name: ss('app_version_choose') })).toBeTruthy();
        expect(screen.queryByRole('button', { name: ss('version_unpin') })).toBeNull();
    });

    it('남겨 둔 버전이 없어도 지금 버전과 다른 버전 선택을 보인다', async () => {
        fakeApi({ updaterVersions: vi.fn().mockResolvedValue({ ...VERSIONS, kept_previous: null }) });
        await openTab();
        await waitFor(() => expect(settings().textContent).toContain(ss('app_version_now', { current: '0.1.14' })));
        expect(settings().textContent).not.toContain('0.1.13');
        expect(screen.getByRole('button', { name: ss('app_version_choose') })).toBeTruthy();
    });

    it('직접 고른 버전이면 고정 표시와 풀리는 버전, 자동 업데이트로 돌아가기를 보인다', async () => {
        fakeApi({ updaterVersions: vi.fn().mockResolvedValue(HELD) });
        await openTab();
        await waitFor(() =>
            expect(settings().textContent).toContain(ss('app_version_held_label', { current: '0.1.13' })),
        );
        expect(row()!.textContent).toContain(ss('version_pinned'));
        expect(row()!.textContent).toContain(ss('version_pinned_note', { until: '0.1.14' }));
        expect(screen.getByRole('button', { name: ss('app_version_choose') })).toBeTruthy();
        expect(screen.getByRole('button', { name: ss('version_unpin') })).toBeTruthy();
    });

    it('버전을 읽지 못하면(옛 데몬 포함) 줄을 그리지 않는다', async () => {
        fakeApi({
            updaterVersions: vi.fn().mockResolvedValue({ ok: false, error: 'Request failed with status code 404' }),
        });
        await openTab();
        await act(async () => {});
        expect(row()).toBeNull();
    });

    it('원격 PC를 보고 있으면 버전을 묻지도 그리지도 않는다', async () => {
        fakeApi({ daemonTarget: vi.fn().mockResolvedValue({ target: 'host-1' }) });
        render(<SabaStorage />);
        await waitFor(() => expect(window.api.daemonTarget).toHaveBeenCalled());
        await act(async () => {});
        expect(window.api.updaterVersions).not.toHaveBeenCalled();
        expect(row()).toBeNull();
    });
});

describe('사바쨩 버전 선택 창', () => {
    it('설치된 버전은 고를 수 없고, 남겨 둔 직전 버전은 언제 설정까지 돌아오는지 알리며, 그보다 옛 버전이 없는 까닭을 적는다', async () => {
        fakeApi({});
        await openDialog();
        expect(screen.getByText(ss('app_version_subtitle', { channel: tr('updates.channel_stable') }))).toBeTruthy();
        const radios = screen.getAllByRole('radio');
        expect(radios.map((r) => r.textContent?.match(/\d+\.\d+\.\d+/)?.[0])).toEqual(['0.1.14', '0.1.13']);
        expect(radios[0]).toHaveProperty('disabled', true);
        expect(radios[0].textContent).toContain(ss('version_installed'));
        expect(radios[0].textContent).toContain(ss('version_latest'));
        expect(radios[1]).toHaveProperty('disabled', false);
        expect(radios[1].textContent).toContain(ss('app_version_previous'));
        expect(radios[1].textContent).toContain(ss('app_version_previous_note', { when }));
        expect(screen.getByText(ss('app_version_older_note', { lowest: '0.1.13' }))).toBeTruthy();
        expect(screen.queryByText(ss('app_version_none'))).toBeNull();
        // 고르기 전에는 바꿀 수 없다
        expect(screen.getByRole('button', { name: ss('app_version_go') })).toHaveProperty('disabled', true);
    });

    it('채널은 선택지의 설명을 뺀 이름만 적는다', async () => {
        fakeApi({ updaterVersions: vi.fn().mockResolvedValue({ ...VERSIONS, channel: 'beta' }) });
        await openDialog();
        const name = tr('updates.channel_beta').split(' (')[0];
        expect(screen.getByText(ss('app_version_subtitle', { channel: name }))).toBeTruthy();
    });

    it('고를 다른 버전이 없으면 그 까닭을 알린다', async () => {
        fakeApi({ updaterVersions: vi.fn().mockResolvedValue({ ...VERSIONS, kept_previous: null }) });
        await openDialog();
        expect(screen.getAllByRole('radio')).toHaveLength(1);
        expect(screen.getByText(ss('app_version_none'))).toBeTruthy();
        expect(screen.getByText(ss('app_version_older_note', { lowest: '0.1.14' }))).toBeTruthy();
    });

    it('고정된 동안 가장 새 버전을 고르면 자동 업데이트로 돌아간다고 알리고 단추도 그렇게 바뀐다', async () => {
        fakeApi({ updaterVersions: vi.fn().mockResolvedValue(HELD) });
        await openDialog();
        const radios = screen.getAllByRole('radio');
        expect(radios.map((r) => r.textContent?.match(/\d+\.\d+\.\d+/)?.[0])).toEqual(['0.1.14', '0.1.13']);
        expect(radios[1].textContent).toContain(ss('version_pinned'));
        expect(radios[0].textContent).toContain(ss('app_version_latest_held_hint'));
        await act(async () => fireEvent.click(radios[0]));
        expect(screen.getByRole('button', { name: ss('version_unpin') })).toHaveProperty('disabled', false);
    });
});

describe('직전 버전으로 되돌리기', () => {
    async function pickPrevious() {
        await openDialog();
        await act(async () => fireEvent.click(screen.getByRole('radio', { name: /0\.1\.13/ })));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: ss('app_version_go') })));
    }

    it('확인 창은 설정이 돌아가는 것, 못 쓰게 되는 구성요소, 고정되는 것을 알리고 바꾸면 업데이터를 띄운다', async () => {
        fakeApi({
            updaterVersions: vi
                .fn()
                .mockResolvedValue({ ...VERSIONS, kept_previous: { ...KEPT, needs_newer_core: ['module-minecraft'] } }),
            updaterRollback: vi.fn().mockResolvedValue({ ok: true, gui_restart: true }),
        });
        await pickPrevious();
        expect(screen.getByText(ss('version_confirm_title', { version: '0.1.13' }))).toBeTruthy();
        const dialog = screen.getByRole('dialog');
        expect(dialog.textContent).toContain(ss('app_version_rollback_message', { version: '0.1.13' }));
        expect(dialog.textContent).toContain(
            ss('app_version_rollback_addons', { version: '0.1.13', list: '마인크래프트 모듈' }),
        );
        expect(dialog.textContent).toContain(ss('app_version_rollback_hold', { current: '0.1.14' }));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: ss('version_confirm_go') })));
        expect(window.api.updaterRollback).toHaveBeenCalled();
        await screen.findByText(ss('app_version_reverting'));
        expect(screen.getByText(ss('app_version_reverting_launched'))).toBeTruthy();
        expect(screen.getByText(ss('app_version_reverting_reopen'))).toBeTruthy();
    });

    it('더 새 버전이 이미 나와 있으면 그 버전까지 고정된다고 알린다', async () => {
        fakeApi({
            updaterVersions: vi.fn().mockResolvedValue({
                ...VERSIONS,
                latest: '0.1.15',
                kept_previous: { ...KEPT, hold_until_newer_than: '0.1.15' },
            }),
        });
        await pickPrevious();
        expect(screen.getByRole('dialog').textContent).toContain(ss('app_version_rollback_hold', { current: '0.1.15' }));
    });

    it('업데이터를 띄우지 못하면 이유와 그대로인 버전을 알린다', async () => {
        fakeApi({
            updaterRollback: vi
                .fn()
                .mockResolvedValue({ ok: false, error: 'saba-chan-installer not found next to the daemon' }),
        });
        await pickPrevious();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: ss('version_confirm_go') })));
        await screen.findByText(
            ss('app_version_failed', {
                version: '0.1.13',
                reason: ss('version_error.updater_not_started'),
                current: '0.1.14',
            }),
        );
        expect(document.body.textContent).not.toContain('not found next to the daemon');
        expect(screen.queryByText(ss('app_version_reverting'))).toBeNull();
    });

    it('데몬이 남겨 둔 버전이 없다고 거절하면 그 까닭으로 알린다', async () => {
        fakeApi({
            updaterRollback: vi
                .fn()
                .mockResolvedValue({
                    ok: false,
                    error: 'No previous version is kept',
                    error_code: 'no_previous_version',
                }),
        });
        await pickPrevious();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: ss('version_confirm_go') })));
        await screen.findByText(
            ss('app_version_failed', {
                version: '0.1.13',
                reason: ss('version_error.no_previous_version'),
                current: '0.1.14',
            }),
        );
    });
});

describe('자동 업데이트로 돌아가기', () => {
    it('고정된 줄의 단추로 확인받고 가장 새 버전으로 다시 시작한다', async () => {
        fakeApi({
            updaterVersions: vi.fn().mockResolvedValue(HELD),
            updaterRestart: vi.fn().mockResolvedValue({ ok: true, gui_restart: true }),
        });
        await openTab();
        await act(async () => fireEvent.click(await screen.findByRole('button', { name: ss('version_unpin') })));
        expect(screen.getByText(ss('version_unpin_title'))).toBeTruthy();
        expect(screen.getByRole('dialog').textContent).toContain(
            ss('app_version_unhold_message', { version: '0.1.14' }),
        );
        expect(screen.getByRole('dialog').textContent).toContain(ss('app_version_unhold_keeps'));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: ss('version_unpin_go') })));
        expect(window.api.updaterRestart).toHaveBeenCalled();
        expect(window.api.updaterRollback).not.toHaveBeenCalled();
    });

    it('받을 새 버전이 없으면 바꾼다고 하지 않고, 고정을 푼 뒤 줄을 다시 읽는다', async () => {
        const updaterVersions = vi
            .fn()
            .mockResolvedValueOnce({ ...HELD, latest: null })
            .mockResolvedValue({ ...HELD, latest: null, held_until_newer_than: null });
        fakeApi({
            updaterVersions,
            updaterRestart: vi.fn().mockResolvedValue({ ok: true, updater_launched: false, daemon_stopping: false }),
        });
        await openTab();
        await act(async () => fireEvent.click(await screen.findByRole('button', { name: ss('version_unpin') })));
        expect(screen.getByRole('dialog').textContent).toContain(ss('app_version_unhold_only'));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: ss('version_unpin_go') })));
        await waitFor(() => expect(row()!.textContent).toContain(ss('app_version_now', { current: '0.1.13' })));
        expect(row()!.textContent).not.toContain(ss('version_pinned'));
    });

    it('창에서 가장 새 버전을 골라도 같은 확인을 거쳐 다시 시작한다', async () => {
        fakeApi({
            updaterVersions: vi.fn().mockResolvedValue(HELD),
            updaterRestart: vi.fn().mockResolvedValue({ ok: true }),
        });
        await openDialog();
        await act(async () => fireEvent.click(screen.getByRole('radio', { name: /0\.1\.14/ })));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: ss('version_unpin') })));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: ss('version_unpin_go') })));
        expect(window.api.updaterRestart).toHaveBeenCalled();
    });

    it('고정되지 않았는데 새 버전이 기다리면 업데이트 알약과 같은 다시 시작 확인을 거친다', async () => {
        fakeApi({
            updaterVersions: vi.fn().mockResolvedValue({ ...VERSIONS, kept_previous: null, latest: '0.1.15' }),
            updaterRestart: vi.fn().mockResolvedValue({ ok: true }),
        });
        await openDialog();
        const latest = screen.getByRole('radio', { name: /0\.1\.15/ });
        expect(latest.textContent).not.toContain(ss('app_version_latest_held_hint'));
        await act(async () => fireEvent.click(latest));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: ss('app_version_go') })));
        expect(screen.getByText(tr('update_pill.restart_title', { version: '0.1.15' }))).toBeTruthy();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: tr('update_pill.restart_now') })));
        expect(window.api.updaterRestart).toHaveBeenCalled();
    });
});

describe('되돌린 뒤 다시 켜졌을 때', () => {
    const REVERTED = {
        ok: true,
        acknowledged: true,
        version: null,
        notes: null,
        result: {
            result: 'reverted',
            version: '0.1.13',
            replaced_by: '0.1.14',
            success: true,
            timestamp: '2026-11-13T01:00:00Z',
            updated_components: [],
            failed: [],
            acknowledged: true,
        },
    };

    beforeEach(() => {
        localStorage.clear();
        window.__sabaNotice!.clearAllNotices();
    });

    it('한 번만 알리고, 업데이트했다고 하지 않는다', async () => {
        fakeApi({ updaterLastResult: vi.fn().mockResolvedValue(REVERTED) });
        const t = i18n.getFixedT(i18n.language, 'gui');
        await announceLastUpdateResult(t);
        const list = window.__sabaNotice!.loadNotices();
        expect(list.map((n) => n.message)).toEqual([ss('app_version_reverted', { from: '0.1.14', version: '0.1.13' })]);
        expect(list[0].type).toBe('success');
        expect(window.api.updaterLastResult).toHaveBeenCalledWith(expect.objectContaining({ includeSeen: true }));

        // 같은 결과로 다시 켜도 더하지 않는다
        useUIStore.setState({ revertResult: null });
        await announceLastUpdateResult(t);
        expect(window.__sabaNotice!.loadNotices()).toHaveLength(1);
        expect(useUIStore.getState().revertResult).toBeNull();
    });

    it('다 되돌리지 못했으면 실패로 알린다', async () => {
        fakeApi({
            updaterLastResult: vi
                .fn()
                .mockResolvedValue({ ...REVERTED, result: { ...REVERTED.result, success: false } }),
        });
        await announceLastUpdateResult(i18n.getFixedT(i18n.language, 'gui'));
        const list = window.__sabaNotice!.loadNotices();
        expect(list.map((n) => n.message)).toEqual([
            ss('app_version_revert_incomplete', { from: '0.1.14', version: '0.1.13' }),
        ]);
        expect(list[0].type).toBe('error');
    });

    it('컴포넌트 탭 위에 같은 문장을 띄우고, 되돌린 버전인 구성요소에는 전 버전을 지워 보인다', async () => {
        fakeApi({
            updaterLastResult: vi.fn().mockResolvedValue(REVERTED),
            updaterVersions: vi.fn().mockResolvedValue(HELD),
        });
        await announceLastUpdateResult(i18n.getFixedT(i18n.language, 'gui'));
        await openTab();
        await screen.findByText(ss('app_version_reverted', { from: '0.1.14', version: '0.1.13' }));
        const core = [...document.querySelectorAll('.ss-card')].find((c) => c.textContent?.includes('Saba-Core'))!;
        await waitFor(() => expect(core.querySelector('s')?.textContent).toBe('v0.1.14'));
        expect(core.querySelector('.ss-card-version')!.textContent).toContain('v0.1.13');
    });

    it('이미 본 다른 결과는 다시 알리지 않는다', async () => {
        fakeApi({
            updaterLastResult: vi.fn().mockResolvedValue({
                ok: true,
                acknowledged: true,
                version: '0.1.14',
                result: { success: true, updated_components: ['gui'], failed: [], acknowledged: true },
            }),
        });
        await announceLastUpdateResult(i18n.getFixedT(i18n.language, 'gui'));
        expect(window.__sabaNotice!.loadNotices()).toHaveLength(0);
    });
});
