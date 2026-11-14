import { useEffect, useState } from 'react';
import type { CatalogModule } from './moduleSupport';

// ─────────────────────────────────────────────────────────────
// 사바 스토리지 목록·아이콘을 앱이 켜져 있는 동안 기억한다.
// 받아 두기는 데몬이 한다(`/api/modules/manifest`·`/api/store/icons`). 여기서는 창을 다시 열 때
// 목록과 그림을 다시 그리지 않고 바로 보이도록 마지막 답을 쥐고 있기만 한다.
// ─────────────────────────────────────────────────────────────

let moduleCatalog: CatalogModule[] | null = null;

/** 마지막으로 받은 모듈 목록 (아직 없으면 null) */
export const cachedModuleCatalog = () => moduleCatalog;

/** 받을 수 있는 모듈 목록. `refresh`(새로고침 버튼)면 데몬도 새로 받는다 */
export async function loadModuleCatalog(
    refresh = false,
): Promise<{ ok: true; modules: CatalogModule[] } | { ok: false; error?: string }> {
    const res = await window.api?.moduleManifest?.(refresh);
    if (res?.success && res.manifest?.modules) {
        const entries = Object.entries(res.manifest.modules as Record<string, Omit<CatalogModule, 'id'>>);
        moduleCatalog = entries.map(([id, info]) => ({ ...info, id }));
        return { ok: true, modules: moduleCatalog };
    }
    return { ok: false, error: res?.error };
}

const icons = new Map<string, string | null>();
const pending = new Map<string, Promise<string | null>>();

/**
 * 아이콘 data URL. 같은 `cacheKey`는 한 번만 부르고 그 뒤로는 바로 준다.
 * `cacheKey`에 버전을 넣으면 새 버전의 그림은 다시 받는다. null이면 부르지 않는다
 */
export function useCachedIcon(cacheKey: string | null, load: () => Promise<string | null> | undefined): string | null {
    const [src, setSrc] = useState(() => (cacheKey ? (icons.get(cacheKey) ?? null) : null));
    // biome-ignore lint/correctness/useExhaustiveDependencies: load는 cacheKey가 같으면 같은 그림을 부른다
    useEffect(() => {
        if (!cacheKey) {
            setSrc(null);
            return;
        }
        if (icons.has(cacheKey)) {
            setSrc(icons.get(cacheKey) ?? null);
            return;
        }
        let live = true;
        let job = pending.get(cacheKey);
        if (!job) {
            job = Promise.resolve(load())
                .then((url) => url ?? null)
                .catch(() => null)
                .then((url) => {
                    icons.set(cacheKey, url);
                    pending.delete(cacheKey);
                    return url;
                });
            pending.set(cacheKey, job);
        }
        job.then((url) => live && setSrc(url));
        return () => {
            live = false;
        };
    }, [cacheKey]);
    return src;
}
