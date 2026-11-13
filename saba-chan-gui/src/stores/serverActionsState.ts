import { useUIStore } from './useUIStore';

/** 화면이 버튼을 눌러 낙관적으로 바꿔 둔 상태 */
interface OptimisticStatus {
    status: string;
    pending?: boolean;
}

/** 동작이 기억하는 것: 화면이 그리지 않으므로 스토어 상태 밖에 둔다 */
const freshInternal = () => ({
    // 이 창이 시킨 켜기·끄기 (폴링이 밖에서 바뀐 것으로 알리지 않는다)
    guiOps: new Set<string>(),
    optimistic: new Map<string, OptimisticStatus>(), // id → { status, pending }
    lastErrorToast: 0,
    firstFetchDone: false,
    // 목록 요청 번호. 먼저 보낸 요청이 늦게 오면 그 목록은 버린다 (G-S4)
    fetchSeq: { sent: 0, applied: 0 },
    bootGrace: true, // suppress notifications during app boot
    bootGraceTimer: null as ReturnType<typeof setTimeout> | null,
});
export let internal = freshInternal();

/** 시험용: 기억한 것을 비운다 (useServerStore._resetForTest) */
export function resetServerActions() {
    if (internal.bootGraceTimer) clearTimeout(internal.bootGraceTimer);
    internal = freshInternal();
}

/** 관리 모드로 켠 서버의 콘솔을 여는 곳. 콘솔은 App의 useMultiConsole이 들고 있어 App이 넣는다 */
export let openConsole: (instanceId: string, serverName: string) => void = () => {};
export const setServerConsole = (open: typeof openConsole) => {
    openConsole = open;
};

export const ui = () => useUIStore.getState();
