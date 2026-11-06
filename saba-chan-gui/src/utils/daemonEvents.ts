import type { TFunction } from 'i18next';
import type { ApiBody } from '../types/daemon';
import type { NoticeInput } from '../types/global';
import { type AuditEntry, auditReason, auditWhat, sourceText } from './audit';

export { sourceText };
import { safeShowToast } from './helpers';

/** 데몬 이벤트 스트림 (P4: GET /api/events → Tauri 셸 → window.api.onDaemonEvent) */
/** 정규화한 이벤트 한 건. data의 칸은 이벤트 종류마다 다르다 */
export interface DaemonEvent {
    id?: unknown;
    type: string;
    data: ApiBody;
}

export interface EventToast {
    message: string;
    type: 'info' | 'warning' | 'error' | 'success';
    duration: number;
    /** 알림 모음에 남길 한 줄 (서버별로 묶는다). 시작·정지·복원·크래시만 남긴다 */
    notice?: NoticeInput;
    /** 토스트는 띄우지 않는다: 앱에서 한 일은 그 자리에서 이미 알렸다 */
    silent?: boolean;
}

export const eventsAvailable = () => typeof window.api?.onDaemonEvent === 'function';

/** 서버 목록을 다시 읽어야 하는 이벤트 (상태는 이벤트로 오지 않는다) */
export const SERVER_CHANGING = ['action.run', 'instance.exit', 'instance.started', 'instance.stopped', 'resync'];

/** preload가 넘긴 한 건 → {id, type, data}. data는 문자열(JSON)이어도 받는다 */
export function normalizeEvent(raw: unknown): DaemonEvent | null {
    if (!raw || typeof raw !== 'object') return null;
    const ev = raw as ApiBody;
    const type = ev.type || ev.event;
    if (typeof type !== 'string') return null;
    let data = ev.data ?? {};
    if (typeof data === 'string') {
        try {
            data = JSON.parse(data);
        } catch {
            data = {};
        }
    }
    return { id: ev.id, type, data: data && typeof data === 'object' ? data : {} };
}

/** 구독하고 해제 함수를 돌려준다. 채널이 없으면 아무것도 하지 않는다 */
export function subscribeDaemonEvents(cb: (ev: DaemonEvent) => void) {
    if (!eventsAvailable()) return () => {};
    const off = window.api.onDaemonEvent((raw: unknown) => {
        const ev = normalizeEvent(raw);
        if (ev) cb(ev);
    });
    return typeof off === 'function' ? off : () => {};
}

/** action.run data(감사 한 줄 모양) → 감사 목록 한 건 모양 */
export const auditEntryOf = (data: ApiBody) => ({ ...data, ts: data.ts ?? data.at });


/**
 * 보낸 명령 한 건(action.run의 stdin·command) → 명령 탭 실행 기록 한 줄. 앱 명령 창이 보낸 것은 그 창이 답과 함께 남기므로 null.
 * via: 콘솔이면 "콘솔", 앱 밖이면 보낸 곳 ("콘솔 · 사바쨩 웹")
 */
export function historyEntryOf(d: ApiBody, t: TFunction) {
    const console = d.action === 'stdin';
    if (!d.instanceId || !d.command || (!console && d.action !== 'command') || (!console && d.source === 'gui')) return null;
    const what = auditWhat({ ...(d as AuditEntry), action: '' }, t);
    const via = [console && t('command_modal.via_console'), d.source !== 'gui' && sourceText(d, t)].filter(Boolean).join(' · ');
    const reason = d.ok ? '' : auditReason(d as AuditEntry, t);
    return {
        id: Date.now() + Math.random(),
        command: what,
        status: d.ok ? 'success' : 'failure',
        via,
        ...(reason ? { detail: reason } : {}),
        time: new Date((d.at ? d.at * 1000 : Date.now())).toLocaleTimeString(),
    };
}

/** instance.exit의 근거 코드 → 이유 한 줄. 콘솔 줄이 근거인 것·모르는 코드는 빈 글자 */
function exitReason(d: ApiBody, t: TFunction) {
    const code = d.evidenceCode;
    if (!code || ['console', 'stop_requested', 'unknown'].includes(code)) return '';
    return t(`events.reason_${code}`, { ...(d.evidenceArgs || {}), defaultValue: '' });
}

/** 다시 켜기 안내: 몇 초 뒤 다시 켬, 자동 재시작 꺼짐, 너무 자주 꺼져 포기함 */
function restartNote(d: ApiBody, t: TFunction) {
    if (d.restartInSecs != null) return t('shell.exit_restart_in', { secs: d.restartInSecs });
    if (d.noteCode === 'auto_restart_off') return t('shell.exit_note_auto_off');
    if (d.noteCode === 'gave_up') return t('shell.exit_note_gave_up', { count: d.noteArgs?.count });
    return '';
}

/** 서버 하나에 묶을 알림 한 줄. 앱에서 한 일은 이미 본 것이라 읽은 것으로 둔다 */
function serverNotice(d: ApiBody, name: string, message: string, type: NoticeInput['type'], detail?: string): NoticeInput {
    return {
        message,
        ...(detail ? { detail } : {}),
        type,
        source: name,
        ...(d.instanceId ? { instanceId: d.instanceId } : {}),
        ...(d.source === 'gui' ? { read: true } : {}),
    };
}

// 켜고 끄는 일은 instance.started·stopped가 출처와 함께 알린다 (action.run은 실패만)
const LIFECYCLE_ACTIONS = ['start', 'stop', 'restart'];

/**
 * 이벤트 → 토스트 {message, type, duration, notice?, silent?} 또는 null.
 * 토스트: GUI에서 직접 한 일은 이미 그 자리에서 띄우므로 다른 곳에서 일어난 일만.
 * 알림 모음(notice): 시작·정지·복원·크래시만, 서버별로 묶을 짧은 줄과 이유 (#14)
 */
export function eventToast(ev: DaemonEvent, t: TFunction, nameOf: (id: string) => string = (id) => id): EventToast | null {
    const d = ev.data;
    switch (ev.type) {
        case 'action.run': {
            const name = d.instance || nameOf(d.instanceId);
            if (d.action === 'restore' && (d.instanceId || d.instance)) {
                const source = sourceText(d, t);
                const reason = auditReason(d as AuditEntry, t);
                return d.ok
                    ? {
                          message: t('events.instance_restored', { name, source }),
                          type: 'success',
                          duration: 5000,
                          notice: serverNotice(d, name, t('events.n_restored', { source }), 'success'),
                          ...(d.source === 'gui' ? { silent: true } : {}),
                      }
                    : {
                          message: t('events.instance_restore_failed', { name }),
                          type: 'error',
                          duration: 8000,
                          notice: serverNotice(d, name, t('events.n_restore_failed'), 'error', reason),
                          ...(d.source === 'gui' ? { silent: true } : {}),
                      };
            }
            // 게임 서버를 대상으로 한 일만 알린다: 길드 설정 바꾸기 같은 일은 작업 기록에만 남는다
            if (d.source !== 'discord' || !(d.instanceId || d.instance)) return null;
            if (d.ok && LIFECYCLE_ACTIONS.includes(d.action)) return null;
            // 출처는 문구에 이미 있다 ("디스코드에서 실행")
            const text = [d.userName || d.user, name, auditWhat(d as AuditEntry, t)].filter(Boolean).join(' · ');
            if (d.ok) return { message: t('events.remote_action', { text }), type: 'info', duration: 5000 };
            const failed = [t('events.remote_action', { text }), t('audit.failed'), auditReason(d as AuditEntry, t)];
            return { message: failed.filter(Boolean).join(' · '), type: 'warning', duration: 6000 };
        }
        case 'instance.exit': {
            if (!['crash', 'start_failed', 'unknown_exit'].includes(d.kind)) return null;
            const name = d.instance || nameOf(d.instanceId);
            const kind = t(`shell.exit_${d.kind}`, { defaultValue: d.kind });
            const reason = exitReason(d, t);
            const restart = d.restartInSecs != null ? ` ${t('shell.exit_restart_in', { secs: d.restartInSecs })}` : '';
            const why = reason ? ` ${t('events.reason_sentence', { reason })}` : '';
            const detail = [reason, restartNote(d, t)].filter(Boolean).join(' · ');
            return {
                message: `${t('events.instance_exit', { name, kind })}${why}${restart}`,
                type: 'warning',
                duration: 8000,
                notice: serverNotice(d, name, t(`events.n_exit_${d.kind}`), 'error', detail || undefined),
            };
        }
        case 'instance.started': {
            const name = d.instance || nameOf(d.instanceId);
            // 사바쨩이 띄우지 않은 프로세스로 켜졌다: 예기치 않은 시작
            if (d.unexpected) {
                return {
                    message: t('events.instance_started_external', { name }),
                    type: 'warning',
                    duration: 10000,
                    notice: serverNotice(d, name, t('events.n_started_external'), 'info'),
                };
            }
            const source = sourceText(d, t);
            return {
                message: t('events.instance_started', { name, source }),
                type: 'info',
                duration: 4000,
                notice: serverNotice(d, name, t('events.n_started', { source }), 'info'),
                ...(d.source === 'gui' ? { silent: true } : {}),
            };
        }
        case 'instance.stopped': {
            const name = d.instance || nameOf(d.instanceId);
            if (d.kind === 'in_game_stop') {
                return {
                    message: t('events.instance_stopped_in_game', { name }),
                    type: 'info',
                    duration: 4000,
                    notice: serverNotice(d, name, t('events.n_stopped_in_game'), 'info'),
                };
            }
            const source = sourceText(d, t);
            return {
                message: t('events.instance_stopped', { name, source }),
                type: 'info',
                duration: 4000,
                notice: serverNotice(d, name, t('events.n_stopped', { source }), 'info'),
                ...(d.source === 'gui' ? { silent: true } : {}),
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
const shownPortMoves = new Set<string>();

/** 데몬이 Windows 예약 범위를 피해 옮겨 저장한 포트를 알린다. 같은 이동은 한 번만 */
export function showPortMoved({ instanceId, name, from, to }: { instanceId: string; name?: string; from: number; to: number }, t: TFunction) {
    const key = `${instanceId}|${from}|${to}`;
    if (shownPortMoves.has(key)) return;
    shownPortMoves.add(key);
    // 알림 모음에는 시작·정지·복원·크래시만 남긴다 (#14): 포트 이동은 토스트로만
    safeShowToast(t('servers.port_moved', { name: name || instanceId, from, to }), 'warning', 8000);
}

// 서버의 예기치 않은 멈춤 하나는 이벤트(instance.exit)와 목록 폴링이 따로 알아챈다.
// 먼저 알린 쪽이 표시를 남기고, 다른 쪽은 그 표시를 지우며 한 번만 건너뛴다 (G-S6).
// 같은 쪽이 연달아 알리면(재시작 사이 크래시가 거듭되면) 모두 알린다
const stopNotices = new Map<string, { source: string; at: number }>();
const STOP_NOTICE_PAIR_MS = 30_000;

/** 이 서버의 예기치 않은 멈춤을 지금 알려도 되는지. source: 'event' | 'poll' */
export function claimStopNotice(instanceId: string | undefined, source: 'event' | 'poll') {
    if (!instanceId) return true;
    const mark = stopNotices.get(instanceId);
    if (mark && mark.source !== source && Date.now() - mark.at < STOP_NOTICE_PAIR_MS) {
        stopNotices.delete(instanceId);
        return false;
    }
    stopNotices.set(instanceId, { source, at: Date.now() });
    return true;
}

/** 토스트를 띄우고, 시작·정지·복원·크래시는 알림 모음에 서버별로 남긴다 */
export function showEventToast(ev: DaemonEvent, t: TFunction, nameOf?: (id: string) => string) {
    const toast = eventToast(ev, t, nameOf);
    if (!toast) return null;
    if (ev.type === 'instance.exit' && !claimStopNotice(ev.data.instanceId, 'event')) return null;
    if (!toast.silent) safeShowToast(toast.message, toast.type, toast.duration);
    if (toast.notice) window.__sabaNotice?.addNotice(toast.notice);
    return toast;
}

/** 서버 하나의 상태가 바뀌었을 때 오는 이벤트 */
const INSTANCE_CHANGED = ['instance.started', 'instance.stopped', 'instance.exit'];

export interface WatchInstanceOptions {
    /** 없으면 이벤트로 깨우지 않고 delay마다 check만 돈다 */
    instanceId?: string;
    check: (ev: DaemonEvent | null) => boolean | Promise<boolean>;
    onTimeout: () => void;
    delay?: number;
    limitMs: number;
}

/**
 * 시작·정지가 끝날 때까지 delay마다 check(ev)를 부른다. check가 true면 끝, limitMs가 지나면 onTimeout.
 * 그 서버의 이벤트가 오면 delay를 기다리지 않고 바로 묻는다. 켜짐 완료처럼 이벤트가 없는 변화도 있어 묻기는 줄이지 않는다.
 * ev는 깨운 이벤트(시간으로 깨면 null)
 */
export function watchInstance({ instanceId, check, onTimeout, delay = 500, limitMs }: WatchInstanceOptions) {
    const deadline = Date.now() + limitMs;
    let done = false;
    let busy = false;
    let pending: DaemonEvent | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let off = () => {};
    const finish = () => {
        done = true;
        clearTimeout(timer);
        off();
    };
    const run = async (ev: DaemonEvent | null): Promise<void> => {
        if (done) return;
        if (busy) {
            pending = ev ?? pending;
            return;
        }
        busy = true;
        clearTimeout(timer);
        let finished = false;
        try {
            finished = await check(ev);
        } catch {
            /* 다음 확인에서 다시 */
        }
        busy = false;
        if (done) return;
        if (finished) return finish();
        if (pending) {
            const next = pending;
            pending = null;
            return run(next);
        }
        if (Date.now() >= deadline) {
            finish();
            return onTimeout();
        }
        timer = setTimeout(() => run(null), delay);
    };
    off = subscribeDaemonEvents((ev) => {
        if (INSTANCE_CHANGED.includes(ev.type) && ev.data?.instanceId === instanceId) run(ev);
    });
    timer = setTimeout(() => run(null), delay);
    return finish;
}
