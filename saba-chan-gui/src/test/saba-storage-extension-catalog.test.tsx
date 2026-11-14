/**
 * 익스텐션 탭 (리뉴얼 시안 2026-11-14): 받을 수 있는 목록을 ▾ 뒤에 숨기지 않고 늘 펼쳐 보인다.
 * 목록에는 앱 언어의 설명과 데몬이 받아 둔 그림을 쓴다. 업데이트는 구성요소 탭의 단추 하나가 맡아
 * 카드마다 업데이트 단추를 두지 않는다.
 */
import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import i18n from '../i18n';

let ctx: Record<string, unknown>;
vi.mock('../contexts/ExtensionContext', () => ({ useExtensions: () => ctx }));

import { ExtensionsTab } from '../components/Modals/SabaStorageExtensions';

const ICON = 'data:image/png;base64,iVBORw0KGgo=';

beforeEach(async () => {
    await i18n.changeLanguage('ko');
    ctx = {
        extensions: [{ id: 'steamcmd', name: 'SteamCMD', version: '1.0.0', enabled: true, has_icon: true }],
        availableUpdates: [{ id: 'steamcmd', latest_version: '1.1.0' }],
        manifestExtensions: [
            { id: 'steamcmd', name: 'SteamCMD', version: '1.1.0', has_icon: true },
            {
                id: 'pet',
                name: 'Desktop Pet',
                version: '0.2.0',
                description: 'A desktop pet',
                descriptions: { ko: '바탕화면 펫' },
                has_icon: true,
            },
        ],
        installingIds: new Set(),
        loading: false,
        refreshExtensions: vi.fn(),
        fetchManifest: vi.fn().mockResolvedValue(null),
        installExtension: vi.fn(),
        removeExtension: vi.fn(),
        toggleExtension: vi.fn(),
    };
    window.api = new Proxy<Record<string | symbol, unknown>>(
        {
            storeIcon: vi.fn().mockResolvedValue(ICON),
            extensionIcon: vi.fn().mockResolvedValue(ICON),
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

const store = () => document.querySelector<HTMLElement>('[data-spot="extensions.store"]')!;

describe('익스텐션 탭', () => {
    it('받을 수 있는 목록이 펼쳐진 채로 앱 언어 설명과 받아 둔 그림을 보인다', async () => {
        render(<ExtensionsTab />);
        expect(ctx.fetchManifest).toHaveBeenCalled();
        expect(store().querySelector('.saba-disclosure-body')).toHaveProperty('hidden', false);
        const cards = store().querySelectorAll('.ss-catalog-card');
        expect(cards).toHaveLength(1);
        expect(cards[0]).toHaveTextContent('바탕화면 펫');
        await waitFor(() => expect(cards[0].querySelector('img')?.getAttribute('src')).toBe(ICON));
        expect(window.api.storeIcon).toHaveBeenCalledWith('ext-pet');
    });

    it('설치된 카드에는 업데이트 단추가 없고 새 버전만 알린다', () => {
        render(<ExtensionsTab />);
        const installed = document.querySelector<HTMLElement>('[data-spot="extensions.installed"]')!;
        expect(installed.querySelector('.saba-disclosure-body')).toHaveProperty('hidden', false);
        expect(installed.querySelector('.ss-icon-btn.accent')).toBeNull();
        expect(installed.querySelector('.ss-update-badge')).toHaveTextContent('v1.1.0');
    });

    it('새로고침은 데몬도 새로 받게 한다', async () => {
        render(<ExtensionsTab />);
        await act(async () => fireEvent.click(store().querySelector('.saba-disclosure-actions button')!));
        expect(ctx.fetchManifest).toHaveBeenLastCalledWith(true);
    });
});
