/**
 * 사바 스토리지 모듈 탭 (맥판 시안 A 2026-11-09, 리뉴얼 시안 2026-11-14):
 * 설치됨·설치 가능·이 기기에서 실행 불가 묶음을 늘 보이고 처음엔 모두 펼쳐 둔다.
 * 그림은 데몬이 받아 둔 것을 쓰고(`storeIcon`), 한 번 받은 그림은 다시 묻지 않는다.
 * 이 데몬의 OS·CPU에서 돌지 않는 모듈은 설치 버튼 없이 이유와 함께 둔다.
 */
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import i18n from '../i18n';

vi.mock('../contexts/ExtensionContext', () => ({
    useExtensions: () => ({ extensions: [], manifestExtensions: [], availableUpdates: [], installingIds: new Set(), loading: false, refresh: vi.fn() }),
}));

import SabaStorage from '../components/Modals/SabaStorage';

const ICON = 'data:image/png;base64,iVBORw0KGgo=';

const CATALOG = {
    minecraft: {
        display_name: 'Minecraft',
        display_names: { ko: '마인크래프트' },
        description: 'Minecraft server management module',
        descriptions: { ko: '마인크래프트 서버 관리 모듈' },
        has_icon: true,
        version: '0.1.12',
        size_bytes: 713031,
        platform_supported: true,
    },
    palworld: {
        display_name: 'Palworld',
        description: 'Palworld server management module',
        architectures: ['x86_64'],
        version: '0.1.12',
        platform_supported: false,
        unsupported_reason: 'arch',
    },
    terraria: {
        display_name: 'Terraria',
        platforms: ['windows', 'linux'],
        version: '0.1.5',
        platform_supported: false,
        unsupported_reason: 'os',
    },
};

beforeEach(async () => {
    await i18n.changeLanguage('ko');
    window.showToast = vi.fn();
    window.api = new Proxy<Record<string | symbol, unknown>>(
        {
            moduleList: vi.fn().mockResolvedValue({ modules: [] }),
            moduleManifest: vi.fn().mockResolvedValue({ ok: true, manifest: { modules: CATALOG } }),
            storeIcon: vi.fn().mockResolvedValue(ICON),
        },
        {
            get(target, prop) {
                if (typeof prop === 'symbol') return target[prop];
                if (!(prop in target)) target[prop] = vi.fn().mockResolvedValue({});
                return target[prop];
            },
        },
    ) as unknown as typeof window.api;
});

afterEach(async () => {
    delete (window as { api?: unknown }).api;
    await i18n.changeLanguage('en');
});

async function openModules() {
    render(<SabaStorage />);
    await act(async () => fireEvent.click(document.querySelectorAll('.saba-storage-tab')[1]));
    await waitFor(() => expect(document.querySelector('.ss-catalog-card')).not.toBeNull());
}

const section = (title: string) =>
    [...document.querySelectorAll<HTMLElement>('.ss-section')].find((el) =>
        el.querySelector('.saba-disclosure-head')?.textContent?.includes(i18n.t(`gui:saba_storage.${title}`)),
    );

describe('받을 수 있는 모듈 목록', () => {
    it('누르지 않아도 목록을 받아 앱 언어의 이름·설명과 데몬이 받아 둔 그림으로 보인다', async () => {
        await openModules();
        const card = document.querySelector('.ss-catalog-card')!;
        expect(card).toHaveTextContent('마인크래프트');
        expect(card).toHaveTextContent('마인크래프트 서버 관리 모듈');
        await waitFor(() => expect(card.querySelector('img')?.getAttribute('src')).toBe(ICON));
        expect(window.api.storeIcon).toHaveBeenCalledWith('module-minecraft');
        expect(card.querySelector('.ss-install-btn')).not.toBeNull();
        // 그림이 없다고 한 모듈은 묻지 않는다
        expect(window.api.storeIcon).toHaveBeenCalledTimes(1);

        // 다시 열면 받아 둔 목록·그림을 바로 그리고, 그림은 다시 묻지 않는다
        cleanup();
        render(<SabaStorage initialTab="modules" />);
        const again = document.querySelector('.ss-catalog-card');
        expect(again).toHaveTextContent('마인크래프트');
        expect(again?.querySelector('img')?.getAttribute('src')).toBe(ICON);
        expect(window.api.storeIcon).toHaveBeenCalledTimes(1);
        await waitFor(() => expect(window.api.moduleManifest).toHaveBeenCalledTimes(2));
    });

    it('묶음은 처음부터 모두 펼쳐져 있고, 돌지 않는 모듈은 설치 버튼 없이 이유와 함께 둔다', async () => {
        await openModules();
        for (const title of ['section_installed', 'section_available', 'section_unsupported']) {
            const el = section(title);
            expect(el, title).toBeDefined();
            expect(el!.querySelector('.saba-disclosure-body')).toHaveProperty('hidden', false);
        }
        expect(document.querySelectorAll('.ss-catalog-card')).toHaveLength(1);
        const group = section('section_unsupported')!;
        expect(group.querySelector('.ss-section-count')).toHaveTextContent('2');
        expect(group.querySelector('.ss-install-btn')).toBeNull();
        const rows = group.querySelectorAll('.ss-unsupported-row');
        expect(rows[0]).toHaveTextContent('Palworld');
        expect(rows[0]).toHaveTextContent(i18n.t('gui:add_server_modal.arch_unsupported'));
        expect(rows[1]).toHaveTextContent(i18n.t('gui:add_server_modal.platform_unsupported'));
    });

    it('목록 새로고침은 데몬도 새로 받게 한다', async () => {
        await openModules();
        const refresh = section('section_available')!.querySelector<HTMLButtonElement>('.saba-disclosure-actions button')!;
        await act(async () => fireEvent.click(refresh));
        expect(window.api.moduleManifest).toHaveBeenLastCalledWith(true);
        // 머리의 단추는 묶음을 접지 않는다
        expect(section('section_available')!.querySelector('.saba-disclosure-body')).toHaveProperty('hidden', false);
    });

    it('모두 이 기기에서 돌면 실행 불가 묶음을 그리지 않는다', async () => {
        vi.mocked(window.api.moduleManifest).mockResolvedValue({ ok: true, manifest: { modules: { minecraft: CATALOG.minecraft } } });
        await openModules();
        await waitFor(() => expect(section('section_unsupported')).toBeUndefined());
    });
});
