/**
 * 익스텐션 화면이 쓰는 공식 통로 `host`. slot 컴포넌트는 props로 받는다.
 * 창마다 할 수 있는 일(서버 켜고 끄기, 앱 열기)은 그 창이 HostWindowContext로 넣고,
 * 데몬 이벤트·번역은 여기서 붙인다. 계약은 docs/architecture/03-extensions.md §4.5
 */
import type { TFunction } from 'i18next';
import { createContext, useContext, useMemo } from 'react';
import i18n from '../i18n';
import type { ServerInfo } from '../types/daemon';
import { type DaemonEvent, subscribeDaemonEvents } from '../utils/daemonEvents';

export const HOST_VERSION = 1;

export type ServerAction = 'start' | 'stop' | 'restart';

/** 창이 정하는 부분 */
export interface HostWindow {
    /** 지금 알고 있는 서버 목록 */
    servers: () => ServerInfo[];
    /** 서버 켜기·끄기·재시작. 확인이 필요하면 그 창의 확인 규칙을 그대로 거친다. 넘겼으면 true (메인 창은 확인 창을 띄운 것도 true) */
    run: (instanceId: string, action: ServerAction) => Promise<boolean>;
    /** 메인 창의 그 화면(`servers`·`discord`·`activity`·`settings`·`ext:<id>`)이나 그 서버(`server:<인스턴스 id>`)를 연다 */
    openApp: (route?: string) => void;
}

export interface ExtensionHost extends HostWindow {
    version: typeof HOST_VERSION;
    /** 이 host를 받은 익스텐션 id */
    extension: string;
    /** 익스텐션 자기 번역(`ext_<id>` 네임스페이스) */
    t: TFunction;
    /** 데몬 이벤트 구독. type `*`는 전부. 이 창이 이벤트를 받지 않으면(트레이 팝업) 부르지 않는다. 끊는 함수를 돌려준다 */
    on: (type: string, fn: (data: DaemonEvent['data'], type: string) => void) => () => void;
    /** manifest `gui.assets` 폴더의 파일(그 폴더 기준 이름)을 그릴 수 있는 URL로. 처음 부를 때 받고 이 창에서 기억한다. 없으면 null */
    assetUrl: (path: string) => Promise<string | null>;
}

/** openApp이 받는 메인 창 화면. 그 밖의 값은 무시한다 (빈 화면으로 가지 않게) */
const APP_ROUTES = ['servers', 'discord', 'activity', 'settings'];
export const isAppRoute = (route: unknown): route is string =>
    typeof route === 'string' &&
    (APP_ROUTES.includes(route) || /^ext:[A-Za-z0-9_-]+$/.test(route) || serverRoute(route) !== null);
/** `server:<인스턴스 id>`의 id */
export const serverRoute = (route: string): string | null => /^server:([A-Za-z0-9_-]{1,128})$/.exec(route)?.[1] ?? null;

const NO_WINDOW: HostWindow = {
    servers: () => [],
    run: async () => false,
    openApp: () => {},
};

export const HostWindowContext = createContext<HostWindow>(NO_WINDOW);

/** 데몬은 정적 파일의 형식을 보지 않는다: 이름으로 붙인다 (그 밖은 브라우저가 내용으로 안다) */
const ASSET_TYPES: Record<string, string> = {
    webp: 'image/webp',
    png: 'image/png',
    jpg: 'image/jpeg',
    jpeg: 'image/jpeg',
    gif: 'image/gif',
    svg: 'image/svg+xml',
    avif: 'image/avif',
};
/** 익스텐션 id → 이름 → 받는 중이거나 받은 URL. 이 창이 닫히거나 익스텐션을 내릴 때까지 둔다 */
const assetUrls = new Map<string, Map<string, Promise<string | null>>>();

function assetUrl(extension: string, path: string): Promise<string | null> {
    let urls = assetUrls.get(extension);
    if (!urls) assetUrls.set(extension, (urls = new Map()));
    const known = urls.get(path);
    if (known) return known;
    const mine = urls;
    const url = window.api.extensionGuiAsset(extension, path).then((base64) => {
        if (typeof base64 !== 'string') {
            // 받지 못했다 (데몬이 아직 안 떴거나 파일이 없다): 다음에 다시 받는다
            mine.delete(path);
            return null;
        }
        const bytes = Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));
        const type = ASSET_TYPES[path.split('.').pop()?.toLowerCase() ?? ''] ?? '';
        return URL.createObjectURL(new Blob([bytes], { type }));
    });
    urls.set(path, url);
    return url;
}

/** 익스텐션을 내리거나 새 판으로 바꿀 때: 받은 URL을 놓는다 (다음 assetUrl은 다시 받는다) */
export function forgetAssets(extension: string) {
    const urls = assetUrls.get(extension);
    assetUrls.delete(extension);
    for (const url of urls?.values() ?? []) {
        url.then((u) => u && URL.revokeObjectURL(u));
    }
}

export function makeHost(extension: string, win: HostWindow): ExtensionHost {
    return {
        ...win,
        version: HOST_VERSION,
        extension,
        t: i18n.getFixedT(null, `ext_${extension}`),
        on: (type, fn) =>
            subscribeDaemonEvents((ev) => {
                if (type === '*' || ev.type === type) fn(ev.data, ev.type);
            }),
        assetUrl: (path) => assetUrl(extension, path),
    };
}

export function useExtensionHost(extension: string): ExtensionHost {
    const win = useContext(HostWindowContext);
    return useMemo(() => makeHost(extension, win), [extension, win]);
}
