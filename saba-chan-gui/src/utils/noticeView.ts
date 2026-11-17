import type { TFunction } from 'i18next';
import type { ApiBody } from '../types/daemon';
import type { Notice } from '../types/global';

/** 알림함 한 건을 그리는 규칙 (앱 알림 창과 웹 콘솔 알림 패널이 같다). 화면 없는 순수 함수 */

/** 데몬의 한 건 (GET /api/notices) → 알림 한 칸 */
export const toNotice = (n: ApiBody): Notice => ({
    id: n.id,
    message: n.message ?? '',
    type: n.level === 'success' || n.level === 'error' ? n.level : 'info',
    source: n.source,
    ...(n.instanceId ? { instanceId: n.instanceId } : {}),
    ...(n.detail ? { detail: n.detail } : {}),
    action: n.action ?? null,
    data: n.data ?? null,
    timestamp: new Date((n.at ?? 0) * 1000).toISOString(),
    read: !!n.read,
});

/** 누르면 그 게임 서버를 업데이트하는 알림 (켤 때 안내·자동 업데이트 대기). data: {instanceId, name} */
export const SERVER_UPDATE_ACTION = 'applyServerUpdate';

/** 누르면 그 서버의 화면을 여는 알림의 액션 → 서버 상세 탭 (데몬 `notices.rs`의 `openBackups`·`openSchedule`) */
const SERVER_SCREENS: Record<string, string> = { openBackups: 'maintenance', openSchedule: 'maintenance' };

/** 누르면 열 화면 주소(utils/screenAddress). 화면을 여는 알림이 아니면 null */
export function noticeScreen(n: Notice) {
    const tab = n.action ? SERVER_SCREENS[n.action] : undefined;
    return tab && n.instanceId ? `server/${n.instanceId}/${tab}` : null;
}

/** 누르면 할 일 안내의 문구 키 (gui 이름공간) */
export function noticeHintKey(action: string) {
    if (action === SERVER_UPDATE_ACTION) return 'notice_modal.click_to_update';
    if (action === 'openBackups') return 'notice_modal.open_backups';
    if (action === 'openSchedule') return 'notice_modal.open_schedule';
    return 'notice_modal.click_to_open';
}

/**
 * 서버 알림은 서버 id로, 나머지는 출처로 묶는다. 최근 알림이 있는 묶음이 먼저 (알림은 최신이 먼저다).
 * 묶음 이름은 그 서버의 지금 이름(이름을 바꿨을 수 있다), 없어진 서버면 알릴 때의 이름, 나머지는 데몬이 준 이름("사바쨩")
 */
export function groupNotices(notices: Notice[], serverName: (id: string) => string | undefined) {
    const groups = new Map<string, Notice[]>();
    for (const n of notices) {
        const key = n.instanceId ? `server:${n.instanceId}` : n.source;
        groups.set(key, [...(groups.get(key) || []), n]);
    }
    return [...groups].map(([key, list]) => ({
        key,
        instanceId: list[0].instanceId,
        label: (list[0].instanceId && serverName(list[0].instanceId)) || list[0].source,
        notices: list,
    }));
}

/** 알린 시각: 방금 전 · N분 전 · 오늘은 시:분 · 어제 시:분 · 그 전은 날짜 시:분. t는 gui 이름공간 */
export function noticeTime(isoString: string, t: TFunction, now = new Date()) {
    const d = new Date(isoString);
    const diff = now.getTime() - d.getTime();
    const clock = d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
    if (diff < 60_000) return t('notice_modal.just_now');
    if (diff < 3_600_000) return t('notice_modal.minutes_ago', { count: Math.floor(diff / 60_000) });
    if (d.toDateString() === now.toDateString()) return clock;
    const yesterday = new Date(now);
    yesterday.setDate(yesterday.getDate() - 1);
    if (d.toDateString() === yesterday.toDateString()) return `${t('notice_modal.yesterday')} ${clock}`;
    return `${d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })} ${clock}`;
}
