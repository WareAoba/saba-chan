/**
 * 번역 런타임 읽기 (05 §번역 런타임): 설치 폴더 번역(데몬 API)을 먼저, 키가 없거나 데몬이 없으면 내장본.
 * 번역 판(`locales_version`)이 바뀌면 다시 읽는다.
 */
import i18next from 'i18next';
import { describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { createLocaleBackend, createLocalesVersionWatcher } from '../utils/localeBackend';

const embeddedKo = { title: '내장 제목', nested: { a: '내장 A', b: '내장 B' } };

async function instance(daemon: (lng: string, ns: string) => Promise<Record<string, unknown> | null>) {
    const i18n = i18next.createInstance();
    const backend = createLocaleBackend(async (lng, ns) => (lng === 'ko' && ns === 'gui' ? embeddedKo : null), daemon);
    await i18n.use(backend).init({ lng: 'ko', fallbackLng: false, ns: ['gui'], defaultNS: 'gui' });
    return i18n;
}

describe('번역 백엔드', () => {
    it('데몬 것을 먼저 쓰고, 데몬에 없는 키는 내장본으로 채운다', async () => {
        const i18n = await instance(async () => ({ title: '설치 제목', nested: { a: '설치 A' } }));
        expect(i18n.t('title')).toBe('설치 제목');
        expect(i18n.t('nested.a')).toBe('설치 A');
        expect(i18n.t('nested.b')).toBe('내장 B');
    });

    it('데몬이 없거나 실패하면 내장본을 쓴다', async () => {
        const i18n = await instance(async () => {
            throw new Error('ECONNREFUSED');
        });
        expect(i18n.t('title')).toBe('내장 제목');
        expect((await instance(async () => null)).t('nested.b')).toBe('내장 B');
    });

    it('글이 아닌 값은 받지 않는다 (내장본을 남긴다)', async () => {
        const i18n = await instance(async () => ({ title: { evil: 1 }, nested: 'x', other: 3 }));
        expect(i18n.t('title')).toBe('내장 제목');
        expect(i18n.t('nested.a')).toBe('내장 A');
        expect(i18n.exists('other')).toBe(false);
    });

    it('reloadResources는 데몬 것을 다시 읽는다', async () => {
        let text = '1판';
        const i18n = await instance(async () => ({ title: text }));
        expect(i18n.t('title')).toBe('1판');
        text = '2판';
        await i18n.reloadResources();
        expect(i18n.t('title')).toBe('2판');
    });
});

describe('번역 판 감시', () => {
    it('판이 바뀔 때만 다시 읽는다', () => {
        const reload = vi.fn();
        const w = createLocalesVersionWatcher(reload);
        w.markDaemonServed();
        w.apply('0.1.13'); // 처음 본 판: 이미 데몬 것을 읽었다
        w.apply('0.1.13');
        expect(reload).not.toHaveBeenCalled();
        w.apply('0.1.14');
        expect(reload).toHaveBeenCalledTimes(1);
        w.apply(null); // 설치 번역이 사라짐 → 내장본으로
        expect(reload).toHaveBeenCalledTimes(2);
        w.apply(undefined); // 판을 모르는 예전 데몬: 무시
        expect(reload).toHaveBeenCalledTimes(2);
    });

    it('처음 읽을 때 데몬이 없었으면 처음 본 판에서 다시 읽는다', () => {
        const reload = vi.fn();
        const w = createLocalesVersionWatcher(reload);
        w.apply('0.1.14');
        expect(reload).toHaveBeenCalledTimes(1);
    });
});

describe('번역 브리지', () => {
    const setup = (res: unknown) => {
        const invoke = vi.fn(async () => {
            if (res instanceof Error) throw res;
            return res;
        });
        const dispatch = vi.fn();
        const { api } = createApi({ invoke, hub: { dispatch, on: vi.fn() } as never });
        return { api, invoke, dispatch };
    };

    it('localesGet은 이 PC 데몬의 GET /api/locales/{lang}/{ns}', async () => {
        const { api, invoke } = setup({ status: 200, data: { a: '가' } });
        await expect(api.localesGet('pt-BR', 'gui')).resolves.toEqual({ a: '가' });
        expect((invoke.mock.calls.at(-1) as unknown[])[1]).toMatchObject({
            method: 'GET',
            path: '/api/locales/pt-BR/gui',
            target: 'local',
        });
    });

    it('localesGet은 없거나(404) 데몬이 없으면 null', async () => {
        await expect(setup({ status: 404, data: null }).api.localesGet('ko', 'gui')).resolves.toBeNull();
        await expect(setup({ status: 0, code: 'ECONNREFUSED' }).api.localesGet('ko', 'gui')).resolves.toBeNull();
    });

    it('updaterStatus는 번역 판을 알린다', async () => {
        const { api, dispatch } = setup({ status: 200, data: { ok: true, locales_version: '0.1.14' } });
        await api.updaterStatus();
        expect(dispatch).toHaveBeenCalledWith('locales:version', '0.1.14');
    });
});
