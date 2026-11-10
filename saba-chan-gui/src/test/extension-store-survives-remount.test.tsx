/**
 * 사바 스토리지 익스텐션 탭을 떠났다 돌아와도 설치 중 표시와 받아 둔 매니페스트가 남는다
 */
import { act, render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ExtensionProvider, useExtensions } from '../contexts/ExtensionContext';

describe('익스텐션 스토어 상태', () => {
    it('탭을 다시 열어도 설치 중 표시와 매니페스트가 남는다', async () => {
        let finish: (v: unknown) => void = () => {};
        window.api = {
            ...window.api,
            extensionList: vi.fn().mockResolvedValue({ extensions: [] }),
            extensionFetchManifest: vi.fn().mockResolvedValue({ extensions: [{ id: 'xna' }], updates: [] }),
            extensionInstall: vi.fn().mockImplementation(
                () =>
                    new Promise((r) => {
                        finish = r;
                    }),
            ),
        } as unknown as typeof window.api;
        type Store = ReturnType<typeof useExtensions>;
        const result: { current: Store | null } = { current: null };
        function Consumer() {
            result.current = useExtensions();
            return null;
        }
        // 탭은 Provider 아래에서 열리고 닫힌다: 같은 Provider 안에서 소비자만 바뀐다
        const tree = (on: boolean) => <ExtensionProvider>{on ? <Consumer /> : null}</ExtensionProvider>;
        const { rerender: show } = render(tree(true));
        const rerender = ({ on }: { on: boolean }) => {
            if (!on) result.current = null;
            show(tree(on));
        };
        await act(async () => {
            await result.current?.fetchManifest();
        });
        let pending: Promise<boolean> | undefined;
        act(() => {
            pending = result.current?.installExtension('xna');
        });
        rerender({ on: false });
        rerender({ on: true });
        expect(result.current?.installingIds.has('xna')).toBe(true);
        expect(result.current?.manifestExtensions.map((e) => e.id)).toEqual(['xna']);
        await act(async () => {
            finish({ success: true });
            await pending;
        });
        expect(result.current?.installingIds.has('xna')).toBe(false);
    });
});
