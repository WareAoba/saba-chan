/**
 * 사바 스토리지 (사용자 10-26):
 * - 탭을 옮길 때 목록을 받기 전에 "없음" 그림을 잠깐 띄우지 않는다.
 * - 열 때마다 순서가 바뀌지 않는다 (컴포넌트는 정한 순서, 모듈·익스텐션은 이름순).
 */
import { act, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

let ctx;
vi.mock('../contexts/ExtensionContext', () => ({ useExtensions: () => ctx }));

import SabaStorage, { parseComponents } from '../components/Modals/SabaStorage';

let resolveModules;
beforeEach(() => {
    ctx = {
        extensions: [],
        loading: true,
        refreshExtensions: vi.fn(),
        manifestExtensions: [],
        availableUpdates: [],
        installingIds: new Set(),
        fetchManifest: vi.fn(),
        installExtension: vi.fn(),
        removeExtension: vi.fn(),
        toggleExtension: vi.fn(),
    };
    window.api = new Proxy(
        { moduleList: vi.fn(() => new Promise((r) => (resolveModules = r))) },
        {
            get(target, prop) {
                if (typeof prop === 'symbol') return target[prop];
                if (!(prop in target)) target[prop] = vi.fn().mockResolvedValue({});
                return target[prop];
            },
        },
    );
});
afterEach(() => {
    delete window.api;
});

const empty = () => document.querySelector('.saba-art, .saba-empty');

describe('받기 전에는 없음 그림을 띄우지 않는다', () => {
    it('모듈 탭', async () => {
        render(<SabaStorage onBack={() => {}} isExiting={false} initialTab="modules" />);
        await act(async () => {});
        expect(empty()).toBeNull();
        await act(async () => resolveModules({ modules: [] }));
        expect(empty()).not.toBeNull();
    });

    it('익스텐션 탭 (익스텐션 목록을 아직 받는 중)', async () => {
        render(<SabaStorage onBack={() => {}} isExiting={false} initialTab="extensions" />);
        await act(async () => {});
        expect(empty()).toBeNull();
    });
});

describe('순서가 바뀌지 않는다', () => {
    it('컴포넌트는 정한 순서', () => {
        const keys = parseComponents(
            ['installer', 'cli', 'updater', 'saba-core', 'gui'].map((component) => ({ component })),
        ).map((c) => c.key);
        expect(keys).toEqual(['saba-core', 'gui', 'cli', 'updater', 'installer']);
    });

    it('모듈과 익스텐션은 이름순', async () => {
        ctx.loading = false;
        ctx.extensions = [
            { id: 'xna', name: 'XNA' },
            { id: 'steamcmd', name: 'SteamCMD' },
        ];
        render(<SabaStorage onBack={() => {}} isExiting={false} initialTab="modules" />);
        await act(async () =>
            resolveModules({
                modules: [
                    { name: 'zomboid', path: '/m/zomboid' },
                    { name: 'minecraft', path: '/m/minecraft' },
                ],
            }),
        );
        const names = () =>
            [...document.querySelectorAll('.ss-cards .ss-card-name')].map((n) => n.firstChild.textContent);
        expect(names()).toEqual(['minecraft', 'zomboid']);
        await act(async () => fireEvent.click(document.querySelectorAll('.saba-storage-tab')[2]));
        expect(names()).toEqual(['SteamCMD', 'XNA']);
    });
});
