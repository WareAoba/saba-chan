/**
 * 설치기·업데이터 창(installer/gui/src)의 화면 흐름.
 * 번들러 없이 실리는 페이지라 HTML·스크립트를 jsdom에 그대로 올리고, Tauri 명령·이벤트만 흉내 낸다.
 * 움직임 줄이기를 켜 두면 그림 펼침 없이 바로 다음 단계로 간다.
 */
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { JSDOM, VirtualConsole } from 'jsdom';
import { describe, expect, it } from 'vitest';

const SRC = path.resolve(__dirname, '../../../installer/gui/src');

type Invoke = Record<string, unknown>;
type Listener = (ev: { payload: unknown }) => void;

function mount(page: string, invokes: Invoke) {
    const html = fs.readFileSync(path.join(SRC, page), 'utf8');
    const scripts = [...html.matchAll(/<script src="([^"]+)"><\/script>/g)].map((m) => m[1]);
    const dom = new JSDOM(html.replace(/<script src="[^"]+"><\/script>/g, ''), {
        url: pathToFileURL(path.join(SRC, page)).href,
        runScripts: 'outside-only',
        pretendToBeVisual: true,
        // 캔버스처럼 jsdom에 없는 기능 알림은 빼고, 페이지 오류는 그대로 보인다
        virtualConsole: new VirtualConsole().sendTo(console, { omitJSDOMErrors: true }),
    });
    const w = dom.window as unknown as Window & { eval: (src: string) => unknown };
    const listeners: Record<string, Listener[]> = {};
    const appWindow = {
        show: async () => {},
        close: () => {},
        minimize: () => {},
        setTitle: async () => {},
        setAlwaysOnTop: async () => {},
        setFocus: async () => {},
    };
    w.matchMedia = ((q: string) => ({
        matches: q.includes('reduce'),
        addEventListener() {},
        removeEventListener() {},
    })) as unknown as typeof window.matchMedia;
    (w as unknown as { __TAURI__: unknown }).__TAURI__ = {
        core: {
            invoke: async (cmd: string, args: unknown) => {
                const v = invokes[cmd];
                return typeof v === 'function' ? v(args) : v;
            },
        },
        event: {
            listen: async (name: string, cb: Listener) => {
                (listeners[name] ||= []).push(cb);
                return () => {};
            },
        },
        window: { getCurrentWindow: () => appWindow },
    };
    for (const s of scripts) w.eval(fs.readFileSync(path.join(SRC, s), 'utf8'));
    const emit = (name: string, payload: unknown) => {
        for (const cb of listeners[name] || []) cb({ payload });
    };
    return { doc: w.document, emit, listening: (name: string) => (listeners[name] || []).length > 0 };
}

/** 없으면 시험을 바로 실패시킨다 */
function byId(doc: Document, id: string) {
    const el = doc.getElementById(id);
    if (!el) throw new Error(`#${id} 없음`);
    return el;
}

const wait = (ms = 0) => new Promise((r) => setTimeout(r, ms));
const never = () => new Promise(() => {});

describe('업데이터 창', () => {
    const invokes = {
        get_theme: 'auto',
        get_preferred_language: 'en',
        get_apply_mode: { enabled: true, components: [], relaunch: true },
        start_apply: never,
    };

    it('성공하면 무엇을 바꿨는지 늘어놓지 않고 링만 성공 색으로 바꾼다', async () => {
        const { doc, emit, listening } = mount('update.html', invokes);
        await wait();
        expect(listening('apply:progress')).toBe(true);
        const flow = byId(doc, 'flow');
        emit('apply:progress', { step: 'applying', message: '', percent: 40 });
        emit('apply:progress', { step: 'complete', message: '', percent: 100, applied: ['saba-core', 'saba-chan-gui'] });
        await wait();
        expect(flow.dataset.step).toBe('done');
        expect(flow.classList.contains('failed')).toBe(false);
        expect(byId(doc, 'progress-ring').classList.contains('ok')).toBe(true);
        expect(doc.body.textContent).not.toContain('saba-core');
        expect(doc.querySelector('.toast, .toast-container, .update-complete-banner')).toBeNull();
    });

    it('적용 뒤 앱을 다시 띄우지 못하면 성공으로 닫지 않고 알린다', async () => {
        const { doc, emit, listening } = mount('update.html', invokes);
        await wait();
        expect(listening('apply:relaunch-failed')).toBe(true);
        emit('apply:progress', { step: 'complete', message: '', percent: 100, applied: ['gui'] });
        emit('apply:relaunch-failed', null);
        await wait();
        const flow = byId(doc, 'flow');
        expect(flow.classList.contains('failed')).toBe(true);
        expect(byId(doc, 'progress-ring').classList.contains('bad')).toBe(true);
        expect(byId(doc, 'fail-summary').hidden).toBe(false);
        expect(byId(doc, 'fail-summary').textContent).not.toBe('');
    });

    it('실패하면 요약을 보이고 닫기 버튼을 띄운다', async () => {
        const { doc, emit } = mount('update.html', invokes);
        await wait();
        emit('apply:progress', { step: 'error', message: 'disk full', percent: -1, applied: ['a'], failed: 2 });
        await wait();
        const flow = byId(doc, 'flow');
        expect(flow.dataset.step).toBe('done');
        expect(flow.classList.contains('failed')).toBe(true);
        expect(byId(doc, 'progress-ring').classList.contains('bad')).toBe(true);
        expect(byId(doc, 'fail-summary').hidden).toBe(false);
        expect(byId(doc, 'fail-summary').textContent).toContain('1 applied, 2 failed');
        expect(doc.body.textContent).not.toContain('disk full');
    });
});

describe('설치기 창', () => {
    const invokes = {
        get_preferred_language: 'en',
        get_app_mode: { uninstall: false },
        get_installer_state: { install_path: 'C:\\Saba-chan', components_dir: 'C:\\Saba-data' },
        get_available_modules: [],
        set_install_path: ({ path: p }: { path: string }) => p,
        set_components_dir: ({ path: p }: { path: string }) => p,
        prepare_install: undefined,
        fetch_latest_release: never,
    };

    async function toInstall(extra: Invoke = {}) {
        const page = mount('index.html', { ...invokes, ...extra });
        await wait();
        const flow = byId(page.doc, 'flow');
        expect(flow.dataset.step).toBe('start');
        byId(page.doc, 'btn-next').click();
        expect(flow.dataset.step).toBe('sheet');
        byId(page.doc, 'btn-install').click();
        await wait();
        return { ...page, flow };
    }

    it('[설치]를 누르면 바로 진행 줄이 있는 단계로 넘어간다', async () => {
        const { flow } = await toInstall();
        expect(['expand', 'install']).toContain(flow.dataset.step);
    });

    it('관리자 권한 확인(설치 폴더 준비)이 끝나기 전에는 그림·설치를 시작하지 않는다', async () => {
        const { flow } = await toInstall({ prepare_install: never });
        expect(flow.dataset.step).toBe('sheet');
    });

    it('권한 확인에 실패하면 시트에 남아 이유를 보인다', async () => {
        const { doc, flow } = await toInstall({
            prepare_install: () => Promise.reject({ error_code: 'elevation_failed', message: 'cancelled' }),
        });
        expect(flow.dataset.step).toBe('sheet');
        const err = byId(doc, 'install-error');
        expect(err.hidden).toBe(false);
        expect(err.textContent).not.toBe('');
        expect((byId(doc, 'btn-install') as HTMLButtonElement).disabled).toBe(false);
    });

    it('완료하면 같은 링이 성공 색이 되고 실행 버튼이 나온다', async () => {
        const { doc, emit, flow } = await toInstall();
        emit('install:progress', { step: 'complete', message: 'done', percent: 100, warnings: [] });
        await wait(700);
        expect(flow.dataset.step).toBe('done');
        expect(flow.classList.contains('failed')).toBe(false);
        expect(byId(doc, 'glow-ring').classList.contains('ok')).toBe(true);
        expect(byId(doc, 'btn-launch').hidden).toBe(false);
        expect(byId(doc, 'btn-retry').hidden).toBe(true);
    });

    it('실패하면 링이 실패 색이 되고 다시 시도를 보인다', async () => {
        const { doc, emit, flow } = await toInstall();
        emit('install:progress', { step: 'error', error_code: 'download_failed', message: 'HTTP 500' });
        await wait();
        expect(flow.dataset.step).toBe('done');
        expect(flow.classList.contains('failed')).toBe(true);
        expect(byId(doc, 'glow-ring').classList.contains('bad')).toBe(true);
        expect(byId(doc, 'btn-launch').hidden).toBe(true);
        expect(byId(doc, 'btn-retry').hidden).toBe(false);
    });

    it('설치하는 동안 제목 표시줄의 닫기를 막고, 끝나면 푼다', async () => {
        const { doc, emit } = await toInstall({ prepare_install: undefined });
        expect((byId(doc, 'btn-close') as HTMLButtonElement).disabled).toBe(true);
        emit('install:progress', { step: 'error', error_code: 'download_failed', message: 'HTTP 500' });
        await wait();
        expect((byId(doc, 'btn-close') as HTMLButtonElement).disabled).toBe(false);
    });
});

describe('제거 창', () => {
    const invokes = {
        get_preferred_language: 'en',
        get_app_mode: { uninstall: true },
        get_installer_state: { install_path: 'C:\\Saba-chan' },
        start_uninstall: never,
    };

    it('지우지 못한 경로와 강제 종료 경고를 완료 화면에 남긴다', async () => {
        const { doc, emit } = mount('index.html', invokes);
        await wait();
        byId(doc, 'btn-uninstall').click();
        await wait();
        expect((byId(doc, 'btn-close') as HTMLButtonElement).disabled).toBe(true);
        emit('uninstall:progress', {
            step: 'complete',
            message: 'done',
            percent: 100,
            left: ['C:\\Saba-chan\\locked.dll'],
            warnings: [{ kind: 'server_force_stopped', server: 'Palworld' }],
        });
        await wait();
        expect(byId(doc, 'flow').dataset.step).toBe('done');
        const box = byId(doc, 'complete-warnings');
        expect(box.hidden).toBe(false);
        expect(box.textContent).toContain('locked.dll');
        expect(box.textContent).toContain('Palworld');
        expect(byId(doc, 'warn-wrap').classList.contains('in')).toBe(true);
        expect((byId(doc, 'btn-close') as HTMLButtonElement).disabled).toBe(false);
    });
});
