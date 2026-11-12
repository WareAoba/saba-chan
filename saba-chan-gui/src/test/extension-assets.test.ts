/**
 * 익스텐션 정적 파일 (manifest `gui.assets`): 번들은 `host.assetUrl(이름)`으로 처음 쓸 때 받는다.
 * 데몬 요청은 셸이 토큰을 붙여 대행하므로, host가 받아 이 창에서 그릴 수 있는 URL로 바꿔 준다
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { forgetAssets, makeHost } from '../extensions/host';

const win = { servers: () => [], run: async () => false, openApp: () => {} };
let created: Blob[] = [];

beforeEach(() => {
    created = [];
    URL.createObjectURL = vi.fn((blob: Blob) => {
        created.push(blob);
        return `blob:asset-${created.length}`;
    });
    URL.revokeObjectURL = vi.fn();
});

afterEach(() => {
    forgetAssets('pet');
});

describe('host.assetUrl', () => {
    it('처음 부를 때 한 번 받아 형식을 붙인 URL로 주고, 다시 부르면 같은 URL', async () => {
        window.api.extensionGuiAsset = vi.fn().mockResolvedValue(btoa('RIFF'));
        const host = makeHost('pet', win);
        const [a, b] = await Promise.all([host.assetUrl('sprites/idle.webp'), host.assetUrl('sprites/idle.webp')]);
        expect(a).toBe('blob:asset-1');
        expect(b).toBe(a);
        expect(await makeHost('pet', win).assetUrl('sprites/idle.webp')).toBe(a);
        expect(window.api.extensionGuiAsset).toHaveBeenCalledTimes(1);
        expect(window.api.extensionGuiAsset).toHaveBeenCalledWith('pet', 'sprites/idle.webp');
        expect(created[0].type).toBe('image/webp');
        expect(Array.from(new Uint8Array(await created[0].arrayBuffer()))).toEqual([...'RIFF'].map((c) => c.charCodeAt(0)));
    });

    it('받지 못하면 null이고 다음에 다시 받는다', async () => {
        window.api.extensionGuiAsset = vi.fn().mockResolvedValueOnce(null).mockResolvedValueOnce(btoa('x'));
        const host = makeHost('pet', win);
        expect(await host.assetUrl('a.png')).toBeNull();
        expect(await host.assetUrl('a.png')).toBe('blob:asset-1');
        expect(created[0].type).toBe('image/png');
    });

    it('익스텐션을 내리거나 새 판으로 바꾸면 받은 URL을 놓고 다시 받는다', async () => {
        window.api.extensionGuiAsset = vi.fn().mockResolvedValue(btoa('x'));
        const host = makeHost('pet', win);
        const first = await host.assetUrl('a.webp');
        forgetAssets('pet');
        await Promise.resolve();
        expect(URL.revokeObjectURL).toHaveBeenCalledWith(first);
        expect(await host.assetUrl('a.webp')).toBe('blob:asset-2');
        expect(window.api.extensionGuiAsset).toHaveBeenCalledTimes(2);
    });
});
