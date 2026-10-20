import { auditWhat } from '../components/servers/AuditTab';
import { safeShowToast } from './helpers';

/** 데몬 이벤트 스트림 (P4: GET /api/events → Tauri 셸 → window.api.onDaemonEvent) */
export const eventsAvailable = () => typeof window.api?.onDaemonEvent === 'function';

/** 서버 목록을 다시 읽어야 하는 이벤트 (상태는 이벤트로 오지 않는다) */
export const SERVER_CHANGING = ['action.run', 'instance.exit', 'resync'];

/** preload가 넘긴 한 건 → {id, type, data}. data는 문자열(JSON)이어도 받는다 */
export function normalizeEvent(raw) {
    if (!raw || typeof raw !== 'object') return null;
    const type = raw.type || raw.event;
    if (typeof type !== 'string') return null;
    let data = raw.data ?? {};
    if (typeof data === 'string') {
        try {
            data = JSON.parse(data);
        } catch {
            data = {};
        }
    }
    return { id: raw.id, type, data: data && typeof data === 'object' ? data : {} };
}

/** 구독하고 해제 함수를 돌려준다. 채널이 없으면 아무것도 하지 않는다 */
export function subscribeDaemonEvents(cb) {
    if (!eventsAvailable()) return () => {};
    const off = window.api.onDaemonEvent((raw) => {
        const ev = normalizeEvent(raw);
        if (ev) cb(ev);
    });
    return typeof off === 'function' ? off : () => {};
}

/** action.run data(감사 한 줄 모양) → 감사 목록 한 건 모양 */
export const auditEntryOf = (data) => ({ ...data, ts: data.ts ?? data.at });

/**
 * 이벤트 → 토스트 {message, type, duration} 또는 null.
 * GUI에서 직접 한 일은 이미 그 자리에서 토스트를 띄우므로, 다른 곳에서 일어난 일만 알린다.
 */
export function eventToast(ev, t, nameOf = (id) => id) {
    const d = ev.data;
    switch (ev.type) {
        case 'action.run': {
            if (d.source !== 'discord') return null;
            // 출처는 문구에 이미 있다 ("디스코드에서 실행")
            const text = [d.userName || d.user, d.instance || d.instanceId, auditWhat(d, t)]
                .filter(Boolean)
                .join(' · ');
            return d.ok
                ? { message: t('events.remote_action', { text }), type: 'info', duration: 5000 }
                : {
                      message: `${t('events.remote_action', { text })} · ${t('audit.failed', { status: d.status })}`,
                      type: 'warning',
                      duration: 6000,
                  };
        }
        case 'instance.exit': {
            if (!['crash', 'start_failed', 'unknown_exit'].includes(d.kind)) return null;
            const name = d.instance || nameOf(d.instanceId);
            const kind = t(`shell.exit_${d.kind}`, { defaultValue: d.kind });
            const restart = d.restartInSecs != null ? ` ${t('shell.exit_restart_in', { secs: d.restartInSecs })}` : '';
            return {
                message: `${t('events.instance_exit', { name, kind })}${restart}`,
                type: 'warning',
                duration: 8000,
            };
        }
        case 'backup.done': {
            // 직접 백업은 시킨 쪽(GUI 토스트, 디스코드면 action.run)이 이미 알린다
            if (d.kind === 'manual') return null;
            const name = nameOf(d.instanceId);
            if (!d.ok) {
                return {
                    message: t('events.backup_failed', { name, error: d.error || '' }),
                    type: 'error',
                    duration: 8000,
                };
            }
            const kind = t(`maintenance.kind_${d.kind}`, { defaultValue: d.kind });
            return { message: t('events.backup_done', { name, kind }), type: 'success', duration: 4000 };
        }
        case 'schedule.warn': {
            if (d.action !== 'restart' && d.action !== 'update') return null;
            const name = d.instance || nameOf(d.instanceId);
            return {
                message: t(`events.schedule_${d.action}`, { name, minutes: d.minutesLeft }),
                type: 'info',
                duration: 6000,
            };
        }
        default:
            return null;
    }
}

// 알린 포트 이동 (서버·from·to). 시작 응답과 instance.port_moved 이벤트가 같은 이동을 두 번 알리지 않게 한다
const shownPortMoves = new Set();

/** 데몬이 Windows 예약 범위를 피해 옮겨 저장한 포트를 알린다. 같은 이동은 한 번만 */
export function showPortMoved({ instanceId, name, from, to }, t) {
    const key = `${instanceId}|${from}|${to}`;
    if (shownPortMoves.has(key)) return;
    shownPortMoves.add(key);
    safeShowToast(t('servers.port_moved', { name: name || instanceId, from, to }), 'warning', 8000, {
        isNotice: true,
        source: 'saba-chan',
    });
}

// 서버가 예기치 않게 멈춘 것을 마지막으로 알린 때. 이벤트와 목록 폴링이 같은 멈춤을 두 번 알리지 않게 한다
const stopNotices = new Map();
const STOP_NOTICE_DEDUP_MS = 30_000;

/** 이 서버의 예기치 않은 멈춤을 지금 알려도 되는지. 알려도 되면 알린 것으로 적는다 */
export function claimStopNotice(instanceId) {
    if (!instanceId) return true;
    const last = stopNotices.get(instanceId);
    if (last !== undefined && Date.now() - last < STOP_NOTICE_DEDUP_MS) return false;
    stopNotices.set(instanceId, Date.now());
    return true;
}

/** 토스트를 띄우고 알림 보기에도 남긴다 */
export function showEventToast(ev, t, nameOf) {
    const toast = eventToast(ev, t, nameOf);
    if (toast && ev.type === 'instance.exit' && !claimStopNotice(ev.data.instanceId)) return null;
    if (toast) safeShowToast(toast.message, toast.type, toast.duration, { isNotice: true, source: 'saba-chan' });
    return toast;
}
