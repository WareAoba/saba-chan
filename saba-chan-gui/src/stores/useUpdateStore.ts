import { create } from 'zustand';
import type { ApiBody } from '../types/daemon';

/**
 * 이 PC 데몬의 업데이트 상태 하나. 상단 바 알약과 사바 스토리지가 같이 읽는다.
 * 데몬이 상태가 바뀔 때 `update.status`(상태 조회와 같은 본문), 받는 동안 `update.progress`를 내고,
 * 셸이 이 PC 데몬의 것만 `onUpdateEvent`로 넘긴다(원격 데몬을 골라도). 화면은 주기로 묻지 않는다.
 */

/** 받는 중인 구성요소. 퍼센트는 데몬이 계산한다 (전체 크기를 모르면 null) */
export interface UpdateProgress {
    component: string | null;
    percent: number | null;
    active: boolean;
}

const IDLE: UpdateProgress = { component: null, percent: null, active: false };

interface UpdateStore {
    /** `GET /api/updates/status`·`update.status`의 본문. 아직 못 읽었으면 null */
    status: ApiBody | null;
    progress: UpdateProgress;
    /** 한 번 읽어 둔다. 읽지 못하면(데몬이 내려갔다) null을 돌려주고 마지막 상태는 그대로 둔다 */
    refresh: () => Promise<ApiBody | null>;
}

export const useUpdateStore = create<UpdateStore>()((set) => ({
    status: null,
    progress: IDLE,
    refresh: async () => {
        let res: ApiBody | null = null;
        try {
            res = await window.api?.updaterStatus?.();
        } catch {
            return null;
        }
        if (!res || res.success === false) return null;
        set({ status: res });
        return res;
    },
}));

function receive(ev: { type?: string; data?: ApiBody } | null | undefined) {
    const data = ev?.data;
    if (!data || typeof data !== 'object') return;
    if (ev.type === 'update.status') {
        useUpdateStore.setState({ status: data });
    } else if (ev.type === 'update.progress') {
        useUpdateStore.setState({
            progress: {
                component: typeof data.component === 'string' ? data.component : null,
                percent: typeof data.percent === 'number' ? data.percent : null,
                active: data.active === true,
            },
        });
    }
}

let watchers = 0;
let unsubscribe: (() => void) | null = null;

/**
 * 업데이트 상태를 보는 화면이 붙을 때 부른다: 처음 붙는 화면이 이벤트를 구독하고 한 번 읽는다.
 * 돌려주는 함수로 뗀다. 마지막 화면이 떼면 구독을 풀고 상태를 비운다
 */
export function watchUpdates(): () => void {
    watchers += 1;
    if (watchers === 1) {
        const off = window.api?.onUpdateEvent?.(receive);
        unsubscribe = typeof off === 'function' ? off : null;
        void useUpdateStore.getState().refresh();
    }
    let attached = true;
    return () => {
        if (!attached) return;
        attached = false;
        watchers -= 1;
        if (watchers > 0) return;
        unsubscribe?.();
        unsubscribe = null;
        useUpdateStore.setState({ status: null, progress: IDLE });
    };
}
