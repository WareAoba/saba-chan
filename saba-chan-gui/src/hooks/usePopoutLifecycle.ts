import { useEffect, useRef } from 'react';

/** 콘솔이 살아 있는 상태 (끄는 중에도 마지막 줄까지 보여 준다) */
const LIVE = new Set(['running', 'starting', 'stopping']);
/** 다시 켜졌다고 볼 상태 */
const STARTED = new Set(['running', 'starting']);

type PopoutServer = { id: string; name: string; status: string };

interface UsePopoutLifecycleParams {
    enabled: boolean;
    servers: PopoutServer[];
    popoutIds: string[];
}

/**
 * PIP를 서버 상태에 맞춘다 (메인 창에서만).
 * - PIP가 떠 있던 서버가 꺼지면 PIP를 닫되, 다시 켜면 띄우도록 기억은 남긴다.
 * - 기억된 서버가 켜지면(앱을 처음 켰을 때 이미 켜져 있던 서버 포함) PIP를 다시 띄운다.
 * 크기·위치·최대화는 셸이 기억한다.
 */
export function usePopoutLifecycle({ enabled, servers, popoutIds }: UsePopoutLifecycleParams) {
    const prevRef = useRef<Map<string, string>>(new Map());
    const openRef = useRef(popoutIds);
    openRef.current = popoutIds;
    const serversRef = useRef(servers);
    serversRef.current = servers;
    /** 비동기 응답이 온 뒤 쓸 최신 상태: 아직 켜져 있으면 그 서버 */
    const startedNow = (id: string) => serversRef.current.find((x) => x.id === id && STARTED.has(x.status));

    useEffect(() => {
        if (!enabled) return;
        const api = window.api;
        const prev = prevRef.current;
        const started: PopoutServer[] = [];
        for (const s of servers) {
            const before = prev.get(s.id);
            prev.set(s.id, s.status);
            if (before === s.status) continue;
            if (!LIVE.has(s.status) && openRef.current.includes(s.id)) {
                if (typeof api?.consolePopoutClose === 'function') {
                    // 닫는 동안 서버가 다시 켜졌으면 닫기가 끝난 뒤 다시 띄운다
                    Promise.resolve(api.consolePopoutClose(s.id, true))
                        .then(() => {
                            const now = startedNow(s.id);
                            if (now) return api.consolePopout?.(s.id, now.name);
                        })
                        .catch(() => {});
                }
            } else if (STARTED.has(s.status) && (before === undefined || !STARTED.has(before))) {
                started.push(s);
            }
        }
        if (!started.length || typeof api?.consolePopoutSaved !== 'function') return;
        Promise.resolve(api.consolePopoutSaved()).then(
            (saved: unknown) => {
                if (!Array.isArray(saved)) return;
                for (const s of started) {
                    const now = startedNow(s.id);
                    if (now && saved.includes(s.id) && !openRef.current.includes(s.id)) {
                        Promise.resolve(api.consolePopout?.(s.id, now.name)).catch(() => {});
                    }
                }
            },
            () => {},
        );
    }, [enabled, servers]);
}
