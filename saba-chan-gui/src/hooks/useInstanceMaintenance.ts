import { useEffect } from 'react';
import { create } from 'zustand';
import type { Server } from '../stores/useServerStore';
import type { ApiBody } from '../types/daemon';
import { eventsLive, subscribeDaemonEvents } from '../utils/daemonEvents';

const PROVISION_POLL_MS = 1200;
/** 데몬의 `provision.progress` 이벤트를 받는 동안 놓친 것을 메우는 간격 */
const PROVISION_SLOW_POLL_MS = 10_000;

/**
 * 인스턴스별 "관리" 상태 — 게임 서버 업데이트 정보, 설치(프로비저닝) 진행률, 설치 완성 카드(cooked).
 * 목록 줄이 확인 작업을 돌리고(서버당 한 번), 상세 화면은 같은 값을 읽기만 한다.
 */
interface Maintenance {
    provisionProgress: ApiBody | null;
    cooked: boolean;
    updateInfo: ApiBody | null;
    /** 마지막 업데이트 확인이 실패한 이유 (`reason: check_failed`). 성공하면 null */
    updateCheckError: string | null;
    updatingServer: boolean;
}

interface MaintenanceState {
    byId: Record<string, Partial<Maintenance>>;
    patch: (id: string, partial: Partial<Maintenance>) => void;
}

type SetServers = (updater: (prev: Server[]) => Server[]) => void;

export const useMaintenanceStore = create<MaintenanceState>()((set) => ({
    byId: {},
    patch: (id, partial) => set((state) => ({ byId: { ...state.byId, [id]: { ...state.byId[id], ...partial } } })),
}));

const EMPTY: Maintenance = { provisionProgress: null, cooked: false, updateInfo: null, updateCheckError: null, updatingServer: false };

/** 읽기 전용: 한 인스턴스의 관리 상태 */
export function useMaintenanceOf(id: string) {
    return useMaintenanceStore((s) => s.byId[id]) || EMPTY;
}

/** 확인 작업 실행: 서버마다 정확히 한 곳(목록 줄)에서 호출한다 */
export function useInstanceMaintenance(server: Server) {
    const patch = useMaintenanceStore((s) => s.patch);

    // 데몬이 주기적으로 확인한 결과: 목록에 실려 오므로 여기서는 읽기만 한다
    const checkKey = JSON.stringify(server.update_check ?? null);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 결과가 바뀔 때만 (목록을 받을 때마다 새 객체다)
    useEffect(() => {
        patch(server.id, updateStateOf(server.update_check ?? null));
    }, [server.id, checkKey, patch]);

    // 프로비저닝 진행률 폴링 — server.provisioning이 true인 동안
    useEffect(() => {
        if (!server.provisioning) {
            patch(server.id, { provisionProgress: null });
            return;
        }
        // 새 설치·업데이트가 시작됐다: 예전 완성 카드는 지운다 (이번에 실패하면 다시 보이지 않게)
        patch(server.id, { cooked: false });
        let cancelled = false;
        let finished = false;
        let wake: (() => void) | null = null;
        // 한 번의 진행 상태(조회 답이나 이벤트)를 화면에 싣는다. 끝났으면 true
        const apply = (result: ApiBody | null | undefined) => {
            if (cancelled || finished || !result?.active) return;
            patch(server.id, { provisionProgress: result });
            if (result.done) {
                // 데몬은 성공한 설치 표시를 곧 지운다 — 완성 카드는 사용자가 닫을 때까지 남긴다
                if (!result.error) patch(server.id, { cooked: true });
                finished = true; // 끝났으면 묻기만 멈추고 화면은 유지
                wake?.();
            }
        };
        // 데몬이 단계마다 진행률을 알린다: 이벤트를 받으면 가끔만 묻는다 (놓친 것 메우기)
        const offEvents = subscribeDaemonEvents((ev) => {
            if (ev.type === 'provision.progress' && ev.data?.instance === server.name) {
                const { instance: _instance, ...progress } = ev.data;
                apply({ ...progress, active: true });
            }
        });
        const poll = async () => {
            while (!cancelled && !finished) {
                try {
                    apply(await window.api.instanceProvisionProgress(server.name));
                } catch {
                    // 무시
                }
                if (cancelled || finished) break;
                await new Promise<void>((r) => {
                    const timer = setTimeout(r, eventsLive() ? PROVISION_SLOW_POLL_MS : PROVISION_POLL_MS);
                    wake = () => {
                        clearTimeout(timer);
                        r();
                    };
                });
                wake = null;
            }
        };
        poll();
        return () => {
            cancelled = true;
            offEvents();
            wake?.();
        };
    }, [server.id, server.provisioning, server.name, patch]);
}

/** 데몬의 확인 결과 → 줄에 보일 값. 실패는 경고로, 새 빌드가 있으면 업데이트 버튼으로 */
function updateStateOf(check: ApiBody | null): Pick<Maintenance, 'updateInfo' | 'updateCheckError'> {
    if (check?.reason === 'check_failed') return { updateInfo: null, updateCheckError: String(check.error || '') };
    return { updateInfo: check?.update_available ? check : null, updateCheckError: null };
}

/** 게임 서버 업데이트 적용 (설치 방식 익스텐션; 지원 안 하면 데몬이 update_unsupported) */
export async function applyInstanceUpdate(server: Server, setServers: SetServers) {
    const { patch } = useMaintenanceStore.getState();
    if (!window.api?.instanceApplyUpdate) return;
    patch(server.id, { updatingServer: true });
    try {
        const result = await window.api.instanceApplyUpdate(server.id);
        if (result?.success) {
            patch(server.id, { updateInfo: null });
            setServers((prev) => prev.map((s) => (s.id === server.id ? { ...s, provisioning: true } : s)));
        }
    } catch {
        // 무시 — 다음 확인 때 다시 표시된다
    } finally {
        patch(server.id, { updatingServer: false });
    }
}

/** 설치 완성 카드 닫기 */
export function closeCookedCard(server: Server) {
    useMaintenanceStore.getState().patch(server.id, { cooked: false });
}

/** 설치 진행 표시 닫기. 설치 실패로 롤백됐으면 목록에서도 뺀다 */
export async function dismissInstanceProvision(server: Server, setServers: SetServers) {
    const { patch } = useMaintenanceStore.getState();
    try {
        const result = await window.api.instanceDismissProvision(server.name);
        patch(server.id, { provisionProgress: null });
        if (result?.rolled_back) {
            setServers((prev) => prev.filter((s) => s.name !== server.name));
        }
    } catch {
        /* 무시 */
    }
}
