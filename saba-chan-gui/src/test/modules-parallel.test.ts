import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setServerI18n, useServerStore } from '../stores/useServerStore';

// 모듈마다 번역과 메타데이터를 차례로 기다려, 원격 데몬처럼 왕복이 느리면 게임 이름·명령 별명이 늦게 찼다
describe('모듈 번역·메타데이터 읽기', () => {
    beforeEach(() => {
        vi.useFakeTimers();
        useServerStore.getState()._resetForTest();
    });
    afterEach(() => vi.useRealTimers());

    it('모듈 수와 상관없이 한 번 왕복하는 시간이면 별명이 찬다', async () => {
        const slow = <T,>(value: T) => new Promise<T>((done) => setTimeout(() => done(value), 300));
        const names = ['a', 'b', 'c', 'd'];
        window.api = {
            daemonStatus: vi.fn().mockResolvedValue({ running: true }),
            moduleList: vi.fn().mockResolvedValue({ modules: names.map((name) => ({ name })) }),
            moduleGetLocales: vi.fn((name: string) => slow({ ko: { name } })),
            moduleGetMetadata: vi.fn((name: string) => slow({ metadata: { module_aliases: [name] } })),
        } as unknown as typeof window.api;
        const addResourceBundle = vi.fn();
        setServerI18n(((key: string) => key) as never, { addResourceBundle } as never);
        const done = useServerStore.getState().fetchModules();
        await vi.advanceTimersByTimeAsync(400);
        expect(Object.keys(useServerStore.getState().moduleAliasesPerModule)).toEqual(names);
        expect(addResourceBundle).toHaveBeenCalledTimes(names.length);
        await done;
    });
});
