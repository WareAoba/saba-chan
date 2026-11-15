import type { ApiBody } from '../types/daemon';
import type { Notice } from '../types/global';
import { toNotice } from './noticeView';

/**
 * 알림함은 데몬이 보관한다 (GET /api/notices). 무엇을 넣을지도 데몬이 정한다: 화면은 넣지 않고 받아 그린다.
 * 받아 둔 목록을 그리고, 데몬이 바뀌었다고 알리면(`notices.changed`)·스트림에 다시 붙으면 다시 읽는다.
 * 읽음·지우기는 데몬에 보내고, 응답을 기다리지 않고 화면부터 바꾼다 (실패하면 다시 읽어 데몬 목록으로 돌린다)
 */

/** 알림함이 바뀌었다는 화면 안 이벤트 */
export const NOTICE_UPDATE = 'saba-notice-update';

let cache: Notice[] = [];

const changed = () => window.dispatchEvent(new CustomEvent(NOTICE_UPDATE));

/** 데몬에서 다시 읽는다. 데몬이 없거나 실패하면 받아 둔 목록을 그대로 둔다 */
export async function refreshNotices() {
    const res = await window.api.noticesList();
    if (!res?.success || !Array.isArray(res.notices)) return;
    cache = res.notices.map(toNotice);
    changed();
}

/** 화면이 바꾼 것을 데몬에 보낸다. 실패하면 데몬 목록으로 돌린다 */
function send(request: Promise<ApiBody> | undefined) {
    Promise.resolve(request)
        .then((res) => {
            if (res && res.success === false) return refreshNotices();
        })
        .catch(() => refreshNotices());
}

export const loadNotices = () => cache;

export const getUnreadCount = () => cache.filter((n) => !n.read).length;

export function markAllRead() {
    if (!cache.some((n) => !n.read)) return;
    cache = cache.map((n) => ({ ...n, read: true }));
    changed();
    send(window.api.noticesRead());
}

export function removeNotice(id: number) {
    cache = cache.filter((n) => n.id !== id);
    changed();
    send(window.api.noticeRemove(id));
}

export function clearAllNotices() {
    cache = [];
    changed();
    send(window.api.noticesClear());
}
