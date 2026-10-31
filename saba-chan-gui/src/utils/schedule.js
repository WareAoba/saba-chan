/**
 * 서버별 예약 작업의 초안 ↔ 데몬 형식과 그 검사 (화면 없는 순수 함수).
 * GUI 백업·예약 탭(MaintenanceTab)과 사바쨩 웹이 같이 쓴다.
 */
export const ACTIONS = ['start', 'stop', 'restart', 'save', 'backup', 'announce', 'update'];
// 게임 명령은 위 목록 대신 따로 띄우는 명령 목록에서 고른다
export const COMMAND = 'command';
// 예고(warn_minutes)를 받는 작업 (데몬 problems()와 같다)
export const WARNS = ['restart', 'update', 'stop'];
// 데몬 `scheduler::WEEKDAYS` (월요일부터)
export const WEEKDAYS = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'];
export const MAX_EVERY_DAYS = 365;
export const MODES = ['every', 'daily', 'days', 'weekdays', 'date'];

export function formatBytes(n) {
    if (!Number.isFinite(n) || n < 0) return '—';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    let i = 0;
    let v = n;
    while (v >= 1024 && i < units.length - 1) {
        v /= 1024;
        i += 1;
    }
    return `${i === 0 ? v : v.toFixed(v < 10 ? 1 : 0)} ${units[i]}`;
}

// ── 예약 작업 초안 ↔ 데몬 형식 ──
let draftSeq = 0;
const modeOf = (task) => {
    if (!task.at) return 'every';
    if (task.every_days != null) return 'days';
    if (task.weekdays?.length) return 'weekdays';
    if (task.date) return 'date';
    return 'daily';
};
export function toDraft(task) {
    draftSeq += 1;
    return {
        key: `t${draftSeq}`,
        action: task.action || 'restart',
        mode: modeOf(task),
        every: task.every_minutes != null ? String(task.every_minutes) : '60',
        at: task.at || '04:00',
        everyDays: task.every_days != null ? String(task.every_days) : '2',
        startDate: task.start_date || '',
        weekdays: task.weekdays || [],
        date: task.date || '',
        warn: (task.warn_minutes || []).join(', '),
        message: task.message || '',
        command: task.command || '',
        args: Object.fromEntries(Object.entries(task.args || {}).map(([k, v]) => [k, String(v)])),
        // 저장된 인자 값 (고치지 않은 값은 원래 형식 그대로 보낸다: 다른 곳에서 글자로 저장한 숫자 등)
        savedArgs: task.args || {},
        enabled: task.enabled !== false,
    };
}

const parseWarn = (text) =>
    text
        .split(/[\s,]+/)
        .map((w) => Number(w))
        .filter((n) => Number.isInteger(n) && n > 0);

/** `inputs`: 고른 게임 명령의 입력 정의 (숫자 입력은 숫자로 보낸다) */
export function fromDraft(d, { announce, inputs = [] }) {
    const task = { action: d.action, enabled: d.enabled };
    if (d.mode === 'every') task.every_minutes = Number(d.every);
    else task.at = d.at;
    if (d.mode === 'days') {
        task.every_days = Number(d.everyDays);
        if (d.startDate) task.start_date = d.startDate;
    }
    if (d.mode === 'weekdays') task.weekdays = WEEKDAYS.filter((w) => d.weekdays.includes(w));
    if (d.mode === 'date') task.date = d.date;
    if (WARNS.includes(d.action) && announce) {
        const warn = parseWarn(d.warn);
        if (warn.length) task.warn_minutes = warn;
    }
    if (d.action === 'announce') task.message = d.message.trim();
    if (d.action === COMMAND) {
        task.command = d.command;
        const args = {};
        for (const [name, raw] of Object.entries(d.args)) {
            const value = String(raw).trim();
            if (!value) continue;
            if (name in d.savedArgs && String(d.savedArgs[name]) === value) {
                args[name] = d.savedArgs[name];
                continue;
            }
            const type = inputs.find((i) => i.name === name)?.type;
            args[name] = type === 'number' && Number.isFinite(Number(value)) ? Number(value) : value;
        }
        if (Object.keys(args).length) task.args = args;
    }
    return task;
}

/** 저장 전에 화면에서 바로 알 수 있는 문제 (데몬 problems()와 같은 규칙). `command`: 고른 게임 명령의 정의 */
export function draftProblems(d, t, command) {
    const out = [];
    if (d.mode === 'every') {
        const n = Number(d.every);
        if (!Number.isInteger(n) || n < 1) out.push(t('maintenance.err_every'));
    } else if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(d.at)) {
        out.push(t('maintenance.err_at'));
    }
    if (d.mode === 'days') {
        const n = Number(d.everyDays);
        if (!Number.isInteger(n) || n < 1 || n > MAX_EVERY_DAYS) {
            out.push(t('maintenance.err_every_days', { max: MAX_EVERY_DAYS }));
        }
    }
    if (d.mode === 'weekdays' && !d.weekdays.length) out.push(t('maintenance.err_weekdays'));
    if (d.mode === 'date' && !/^\d{4}-\d{2}-\d{2}$/.test(d.date)) out.push(t('maintenance.err_date'));
    if (d.action === 'announce' && !d.message.trim()) out.push(t('maintenance.err_message'));
    if (d.action === COMMAND) {
        if (!d.command) out.push(t('maintenance.err_command'));
        for (const input of command?.inputs || []) {
            if (input.required && input.default == null && !String(d.args[input.name] ?? '').trim()) {
                out.push(t('maintenance.err_command_input', { name: input.label || input.name }));
            }
        }
    }
    return out;
}

/** 요일 이름 (보는 사람의 언어, 2024-01-01이 월요일) */
export const weekdayLabel = (lang, i) =>
    new Intl.DateTimeFormat(lang, { weekday: 'short', timeZone: 'UTC' }).format(new Date(Date.UTC(2024, 0, 1 + i)));

/** UTC 차이(분) → "UTC+9", "UTC-3:30" */
export const offsetText = (minutes) => {
    const sign = minutes < 0 ? '-' : '+';
    const abs = Math.abs(minutes);
    const h = Math.floor(abs / 60);
    const m = abs % 60;
    return `UTC${sign}${h}${m ? `:${String(m).padStart(2, '0')}` : ''}`;
};
