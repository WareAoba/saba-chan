/**
 * 사바 스토리지 모듈 탭의 받을 수 있는 목록 (맥판 시안 A, 2026-11-09):
 * 목록은 접지 않고 바로 받아 보이고, 아이콘과 앱 언어의 이름·설명을 쓰며,
 * 이 데몬의 OS·CPU에서 돌지 않는 모듈은 설치 버튼 없이 접힌 묶음에 이유와 함께 둔다.
 */
import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import i18n from '../i18n';

vi.mock('../contexts/ExtensionContext', () => ({
    useExtensions: () => ({ extensions: [], manifestExtensions: [], availableUpdates: [], installingIds: new Set(), loading: false, refresh: vi.fn() }),
}));

import SabaStorage from '../components/Modals/SabaStorage';

const CATALOG = {
    minecraft: {
        display_name: 'Minecraft',
        display_names: { ko: '마인크래프트' },
        description: 'Minecraft server management module',
        descriptions: { ko: '마인크래프트 서버 관리 모듈' },
        icon: 'https://raw.githubusercontent.com/x/minecraft/icon.png',
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
            moduleManifest: vi.fn().mockResolvedValue({ success: true, manifest: { modules: CATALOG } }),
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

describe('받을 수 있는 모듈 목록', () => {
    it('누르지 않아도 목록을 받아 앱 언어의 이름·설명과 아이콘으로 보인다', async () => {
        await openModules();
        const card = document.querySelector('.ss-catalog-card')!;
        expect(card).toHaveTextContent('마인크래프트');
        expect(card).toHaveTextContent('마인크래프트 서버 관리 모듈');
        expect(card.querySelector('img')?.getAttribute('src')).toBe(CATALOG.minecraft.icon);
        expect(card.querySelector('.ss-install-btn')).not.toBeNull();
    });

    it('이 기기에서 돌지 않는 모듈은 설치 버튼 없이 접힌 묶음에 이유와 함께 둔다', async () => {
        await openModules();
        expect(document.querySelectorAll('.ss-catalog-card')).toHaveLength(1);
        // 공통 펼치기 (사용자 11-11), 접혀 있다
        const group = document.querySelector('.ss-unsupported') as HTMLElement;
        expect(group).not.toBeNull();
        expect(group.querySelector('.saba-disclosure-body')).toHaveProperty('hidden', true);
        expect(group.querySelector('.saba-disclosure-head')).toHaveTextContent('2');
        expect(group.querySelector('.ss-install-btn')).toBeNull();
        const rows = group.querySelectorAll('.ss-unsupported-row');
        expect(rows[0]).toHaveTextContent('Palworld');
        expect(rows[0]).toHaveTextContent(i18n.t('gui:add_server_modal.arch_unsupported'));
        expect(rows[1]).toHaveTextContent(i18n.t('gui:add_server_modal.platform_unsupported'));
    });

    it('모두 이 기기에서 돌면 접힌 묶음을 그리지 않는다', async () => {
        vi.mocked(window.api.moduleManifest).mockResolvedValue({ success: true, manifest: { modules: { minecraft: CATALOG.minecraft } } });
        await openModules();
        expect(document.querySelector('details.ss-unsupported')).toBeNull();
    });
});
