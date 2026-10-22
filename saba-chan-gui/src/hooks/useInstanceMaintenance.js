import { useEffect } from 'react';
import { create } from 'zustand';
import { extensionInstallMethod } from '../utils/installMethod';

const UPDATE_CHECK_INTERVAL_MS = 3 * 60 * 60 * 1000; // 3시간
const PROVISION_POLL_MS = 1200;

/**
 * 인스턴스별 "관리" 상태 — 게임 서버 업데이트 정보, 설치(프로비저닝) 진행률, 설치 완성 카드(cooked).
 * 목록 줄이 확인 작업을 돌리고(서버당 한 번), 상세 화면은 같은 값을 읽기만 한다.
 */
export const useMaintenanceStore = create((set) => ({
    byId: {},
    patch: (id, partial) => set((state) => ({ byId: { ...state.byId, [id]: { ...state.byId[id], ...partial } } })),
}));

const EMPTY = { provisionProgress: null, cooked: false, updateInfo: null, updatingServer: false };

/** 읽기 전용: 한 인스턴스의 관리 상태 */
export function useMaintenanceOf(id) {
    return useMaintenanceStore((s) => s.byId[id]) || EMPTY;
}

/** 확인 작업 실행: 서버마다 정확히 한 곳(목록 줄)에서 호출한다 */
export function useInstanceMaintenance(server, module) {
    const patch = useMaintenanceStore((s) => s.patch);
    const installMethod = extensionInstallMethod(server, module);

    // 익스텐션 설치 방식으로 설치한 인스턴스의 게임 서버 업데이트 확인 (3시간 주기)
    useEffect(() => {
        if (server.provisioning) return;
        if (!installMethod) return;
        if (!window.api?.instanceCheckUpdate) return;

        let cancelled = false;
        const doCheck = async () => {
            try {
                const result = await window.api.instanceCheckUpdate(server.id);
                if (cancelled || !result) return;
                patch(server.id, { updateInfo: result.update_available ? result : null });
            } catch {
                // 백그라운드 확인 — 실패는 무시
            }
        };
        doCheck();
        const timer = setInterval(doCheck, UPDATE_CHECK_INTERVAL_MS);
        return () => {
            cancelled = true;
            clearInterval(timer);
        };
    }, [server.id, server.provisioning, installMethod, patch]);

    // 프로비저닝 진행률 폴링 — server.provisioning이 true인 동안
    useEffect(() => {
        if (!server.provisioning) {
            patch(server.id, { provisionProgress: null });
            return;
        }
        let cancelled = false;
        const poll = async () => {
            while (!cancelled) {
                try {
                    const result = await window.api.instanceProvisionProgress(server.name);
                    if (cancelled) break;
                    if (result?.active) {
                        patch(server.id, { provisionProgress: result });
                        if (result.done) {
                            // 데몬은 성공한 설치 표시를 곧 지운다 — 완성 카드는 사용자가 닫을 때까지 남긴다
                            if (!result.error) patch(server.id, { cooked: true });
                            break; // 끝났으면 폴링만 멈추고 화면은 유지
                        }
                    }
                } catch {
                    // 무시
                }
                await new Promise((r) => setTimeout(r, PROVISION_POLL_MS));
            }
        };
        poll();
        return () => {
            cancelled = true;
        };
    }, [server.id, server.provisioning, server.name, patch]);
}

/** 게임 서버 업데이트 적용 (설치 방식 익스텐션; 지원 안 하면 데몬이 update_unsupported) */
export async function applyInstanceUpdate(server, setServers) {
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
export function closeCookedCard(server) {
    useMaintenanceStore.getState().patch(server.id, { cooked: false });
}

/** 설치 진행 표시 닫기. 설치 실패로 롤백됐으면 목록에서도 뺀다 */
export async function dismissInstanceProvision(server, setServers) {
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
