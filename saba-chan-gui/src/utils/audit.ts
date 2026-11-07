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

const SOURCES = ['gui', 'cli', 'api', 'discord', 'chatbot', 'schedule', 'daemon', 'web', 'remote', 'mcp', 'restart'];
const ACTIONS = ['start', 'stop', 'restart', 'command', 'stdin', 'backup', 'restore', 'remote', 'crash'];
export const PERIODS = ['all', 'today', '7d', '30d', 'custom'];
const DAYS_BACK: Record<string, number> = { today: 0, '7d': 6, '30d': 29 }; // 오늘을 포함해 며칠

// 이 PC 시간대로 그날 0시 (unix 초)
const dayStart = (date: Date) =>
    Math.floor(new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime() / 1000);
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

/** 작업 기록에 보일 줄: 서버(인스턴스)를 조작한 것과 원격 데몬에 중계한 조작만. 디스코드·말하는 사바쨩 설정 같은 나머지는 숨긴다 */
export const isInstanceEntry = (e: AuditEntry & { instanceId?: string }) => !!e.instanceId || e.action === 'remote';

// 값 하나를 글로: 배열·객체는 안의 값만 띄어 쓴다 (JSON 기호를 보이지 않는다)
const flat = (x: unknown): string[] => {
    if (x === null || x === undefined || x === '') return [];
    if (Array.isArray(x)) return x.flatMap(flat);
    if (typeof x === 'object') return Object.values(x as object).flatMap(flat);
    return [String(x)];
};

// 데몬은 인자를 JSON 문자열로 남긴다 (잘렸을 수 있다). 배열·칸 이름이 붙은 객체는 값만 띄어 쓰고,
// 값이 글 하나뿐인 객체는 따옴표로 묶는다. 빈 인자({}·[])는 보이지 않는다.
// JSON이 아니면(잘림) 괄호·따옴표·칸 이름을 걷어 내고 잘렸다고 표시한다
const argsText = (args: string | undefined) => {
    let v: unknown;
    try {
        v = JSON.parse(args as string);
    } catch {
        if (!args || !/^\s*[[{"]/.test(args)) return args;
        const rest = args
            .replace(/"[^"]*"\s*:/g, ' ')
            .replace(/[[\]{}",]+/g, ' ')
            .replace(/\s+/g, ' ')
            .trim();
        return rest ? `${rest}…` : '';
    }
    const named = !!v && typeof v === 'object' && !Array.isArray(v);
    const values = flat(v);
    if (
        named &&
        values.length === 1 &&
        typeof Object.values(v as object).find((x) => x !== '' && x != null) === 'string'
    )
        return `“${values[0]}”`;
    return values.join(' ');
};

/** 동작 이름: 기본 동작은 audit.action_*, 데몬 액션(instance.settings.set 등)은 액션 표의 제목. 모르면 그대로 */
const actionText = (action: string, t: TFunction) => {
    if (ACTIONS.includes(action)) return t(`audit.action_${action}`);
    return t(`bot:actions.${action}.title`, { defaultValue: '' }) || action;
};

/** 한 건 → "무엇을" 한 줄 (명령이면 명령 이름과 인자, 원격 데몬에 중계한 조작이면 그 데몬과 요청) */
export function auditWhat(e: AuditEntry, t: TFunction) {
    const action = actionText(e.action, t);
    const detail = [e.command, argsText(e.args)].filter(Boolean).join(' ');
    return [action, e.daemon, detail].filter(Boolean).join(' · ');
}

/** 작업 기록 거르기 (여럿 고를 수 있다): 시작·정지·재시작 · 크래시 · 그 외 명령 · 실패 */
export const FILTERS = ['power', 'crash', 'command', 'failed'];
const POWER = ['start', 'stop', 'restart'];
const skipped = (e: AuditEntry) => e.action === 'backup' && e.status === 409 && !e.code;

// 한 건이 거르기 하나에 드는지. 실패는 크래시와 건너뛴 예약 백업을 빼고 센다
function matchesOne(e: AuditEntry, filter: string) {
    switch (filter) {
        case 'power':
            return POWER.includes(e.action);
        case 'crash':
            return e.action === 'crash';
        case 'command':
            return !POWER.includes(e.action) && e.action !== 'crash';
        case 'failed':
            return !e.ok && e.action !== 'crash' && !skipped(e);
        default:
            return false;
    }
}

/** 고른 거르기 중 하나라도 드는지. 아무것도 고르지 않았으면 모두 보인다 */
export function auditMatches(e: AuditEntry, filters: readonly string[]) {
    return filters.length === 0 || filters.some((f) => matchesOne(e, f));
}

/** 한 건 → 결과 칩. 예약 백업은 서버가 꺼져 있으면 409로 건너뛴다. 크래시는 데몬이 남긴 종료 판정이다 */
export function auditResult(e: AuditEntry, t: TFunction) {
    if (e.action === 'crash') return { tone: 'stop', label: t('audit.crashed') };
    if (e.ok) return { tone: 'ok', label: t('audit.ok') };
    if (e.action === 'backup' && e.status === 409 && !e.code)
        return { tone: 'idle', label: t('audit.skipped_stopped') };
    return { tone: 'stop', label: t('audit.failed') };
}

// 실패 이유 코드의 문구를 찾는 곳 (먼저 찾은 것). 값을 채워야 하는 문구({{…}})는 쓰지 않는다
const REASON_KEYS = (code: string) => [
    `audit.code_${code}`,
    `errors.${code}`,
    `maintenance.err_${code}`,
    `common:module_errors.${code}`,
];

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

// 인자 → 펼친 칸의 줄들: 칸 이름이 붙은 객체는 "이름: 값" 한 줄씩, 그 밖은 한 줄 (JSON 기호 없이)
const argLines = (args: string | undefined): string[] => {
    if (!args) return [];
    let v: unknown;
    try {
        v = JSON.parse(args);
    } catch {
        const text = argsText(args);
        return text ? [text] : [];
    }
    if (v && typeof v === 'object' && !Array.isArray(v)) {
        return Object.entries(v as object)
            .map(([k, x]) => [k, flat(x).join(' ')])
            .filter(([, x]) => x)
            .map(([k, x]) => `${k}: ${x}`);
    }
    const text = flat(v).join(' ');
    return text ? [text] : [];
};

/** 한 건 → 펼쳐 보이는 자세한 칸 [이름, 값]. 빈 칸은 뺀다 */
export function auditDetails(
    e: AuditEntry & { ts?: number; instance?: string; instanceId?: string },
    t: TFunction,
    locale?: string,
) {
    const rows: Array<[string, string]> = [
        [t('audit.detail_time'), e.ts ? new Date(e.ts * 1000).toLocaleString(locale) : ''],
        [t('audit.detail_server'), [e.instance, e.instanceId].filter(Boolean).join(' · ')],
        [t('audit.detail_action'), actionText(e.action, t)],
        [t('audit.detail_daemon'), e.daemon || ''],
        [t('audit.detail_command'), e.command || ''],
        ...argLines(e.args).map((line): [string, string] => [t('audit.detail_args'), line]),
        [t('audit.detail_source'), SOURCES.includes(e.source) ? t(`audit.source_${e.source}`) : e.source],
        [
            t('audit.detail_user'),
            [e.userName, e.user && e.user !== e.userName ? e.user : ''].filter(Boolean).join(' · '),
        ],
        [
            t('audit.detail_guild'),
            [e.guildName, e.guild && e.guild !== e.guildName ? e.guild : ''].filter(Boolean).join(' · '),
        ],
        [t('audit.detail_result'), auditResult(e, t).label],
        [t('audit.detail_reason'), [auditReason(e, t), e.code].filter(Boolean).join(' · ')],
        [t('audit.detail_status'), e.status && e.action !== 'crash' ? String(e.status) : ''],
    ];
    return rows.filter(([, v]) => v);
}
