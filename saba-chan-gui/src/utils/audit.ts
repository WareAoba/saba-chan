import type { TFunction } from 'i18next';

// 활동 기록(데몬 GET /api/audit) 한 건을 문구로 바꾸는 순수 함수. 앱과 웹이 같이 쓴다 (문구는 gui 네임스페이스 audit.*)

/** 활동 기록 한 건 (audit::Event + Origin 칸) */
export interface AuditEntry {
    action: string;
    command?: string;
    args?: string;
    daemon?: string;
    ok?: boolean;
    status?: number;
    /** 실패 이유 코드 (데몬 응답의 error_code) */
    code?: string;
    source: string;
    user?: string;
    userName?: string;
    guild?: string;
    guildName?: string;
}

const SOURCES = ['gui', 'cli', 'api', 'discord', 'chatbot', 'schedule', 'daemon', 'web', 'remote', 'mcp', 'agent', 'restart'];
const ACTIONS = ['start', 'stop', 'restart', 'command', 'stdin', 'backup', 'restore', 'remote'];
export const PERIODS = ['all', 'today', '7d', '30d', 'custom'];
const DAYS_BACK: Record<string, number> = { today: 0, '7d': 6, '30d': 29 }; // 오늘을 포함해 며칠

// 이 PC 시간대로 그날 0시 (unix 초)
const dayStart = (date: Date) => Math.floor(new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime() / 1000);
// "2026-10-01" → 그날 0시. 비었거나 틀리면 null
const parseDay = (value: string | undefined, plusDays = 0) => {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value || '');
    return m ? dayStart(new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]) + plusDays)) : null;
};

/** 기간 → 데몬 조회 범위 {since?, until?} (until은 그 시각 전까지) */
export function auditRange(period: string, from?: string, to?: string): { since?: number; until?: number } {
    if (period in DAYS_BACK) {
        const now = new Date();
        return { since: dayStart(new Date(now.getFullYear(), now.getMonth(), now.getDate() - DAYS_BACK[period])) };
    }
    if (period !== 'custom') return {};
    const since = parseDay(from);
    const until = parseDay(to, 1); // 끝 날짜는 그날 끝까지
    return { ...(since !== null ? { since } : {}), ...(until !== null ? { until } : {}) };
}

// 데몬은 인자를 JSON 문자열로 남긴다 (잘렸을 수 있다). 배열은 띄어 쓰고, 칸 이름이 붙은 객체는 값만 띄어 쓰고,
// 값이 글 하나뿐이면 따옴표로 묶는다. 빈 인자({}·[])는 보이지 않는다. JSON이 아니면(잘림) 그대로
const argsText = (args: string | undefined) => {
    let v: unknown;
    try {
        v = JSON.parse(args as string);
    } catch {
        return args;
    }
    const named = !!v && typeof v === 'object' && !Array.isArray(v);
    const values = (Array.isArray(v) ? v : named ? Object.values(v as object) : [v]).filter((x) => x !== null && x !== undefined && x !== '');
    if (named && values.length === 1 && typeof values[0] === 'string') return `“${values[0]}”`;
    return values.map((x) => (typeof x === 'object' ? JSON.stringify(x) : String(x))).join(' ');
};

/** 한 건 → "무엇을" 한 줄 (명령이면 명령 이름과 인자, 원격 데몬에 중계한 조작이면 그 데몬과 요청) */
export function auditWhat(e: AuditEntry, t: TFunction) {
    const action = ACTIONS.includes(e.action) ? t(`audit.action_${e.action}`) : e.action;
    const detail = [e.command, argsText(e.args)].filter(Boolean).join(' ');
    return [action, e.daemon, detail].filter(Boolean).join(' · ');
}

/** 한 건 → 결과 칩. 예약 백업은 서버가 꺼져 있으면 409로 건너뛴다 */
export function auditResult(e: AuditEntry, t: TFunction) {
    if (e.ok) return { tone: 'ok', label: t('audit.ok') };
    if (e.action === 'backup' && e.status === 409 && !e.code) return { tone: 'idle', label: t('audit.skipped_stopped') };
    return { tone: 'stop', label: t('audit.failed') };
}

// 실패 이유 코드의 문구를 찾는 곳 (먼저 찾은 것). 값을 채워야 하는 문구({{…}})는 쓰지 않는다
const REASON_KEYS = (code: string) => [`audit.code_${code}`, `errors.${code}`, `maintenance.err_${code}`, `common:module_errors.${code}`];

/** 실패한 한 건 → 이유 한 줄 (모르는 코드면 빈 글자) */
export function auditReason(e: Pick<AuditEntry, 'ok' | 'code'>, t: TFunction) {
    if (e.ok || !e.code) return '';
    for (const key of REASON_KEYS(e.code)) {
        const text = t(key, { defaultValue: '' });
        if (text && !text.includes('{{')) return text;
    }
    return '';
}

/** 시킨 쪽(감사 출처) → "원격 PC · 집 PC" (instance.started·stopped, 콘솔에 보낸 줄) */
export function sourceText(d: { source?: string; userName?: string; [key: string]: unknown }, t: TFunction) {
    const label = t(`audit.source_${d.source}`, { defaultValue: d.source || '' });
    return d.userName ? `${label} · ${d.userName}` : label;
}

/** 한 건 → "누가·어디서" 한 줄 */
export function auditWho(e: AuditEntry, t: TFunction) {
    const source = SOURCES.includes(e.source) ? t(`audit.source_${e.source}`) : e.source;
    return [source, e.userName || e.user, e.guildName || e.guild].filter(Boolean).join(' · ');
}
