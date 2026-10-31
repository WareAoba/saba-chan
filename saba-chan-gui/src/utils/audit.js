// 활동 기록(데몬 GET /api/audit) 한 건을 문구로 바꾸는 순수 함수. 앱과 웹이 같이 쓴다 (문구는 gui 네임스페이스 audit.*)

const SOURCES = ['gui', 'cli', 'api', 'discord', 'schedule', 'daemon', 'web', 'remote', 'mcp', 'restart'];
const ACTIONS = ['start', 'stop', 'restart', 'command', 'stdin', 'backup', 'remote'];
export const PERIODS = ['all', 'today', '7d', '30d', 'custom'];
const DAYS_BACK = { today: 0, '7d': 6, '30d': 29 }; // 오늘을 포함해 며칠

// 이 PC 시간대로 그날 0시 (unix 초)
const dayStart = (date) => Math.floor(new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime() / 1000);
// "2026-10-01" → 그날 0시. 비었거나 틀리면 null
const parseDay = (value, plusDays = 0) => {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value || '');
    return m ? dayStart(new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]) + plusDays)) : null;
};

/** 기간 → 데몬 조회 범위 {since?, until?} (until은 그 시각 전까지) */
export function auditRange(period, from, to) {
    if (period in DAYS_BACK) {
        const now = new Date();
        return { since: dayStart(new Date(now.getFullYear(), now.getMonth(), now.getDate() - DAYS_BACK[period])) };
    }
    if (period !== 'custom') return {};
    const since = parseDay(from);
    const until = parseDay(to, 1); // 끝 날짜는 그날 끝까지
    return { ...(since !== null ? { since } : {}), ...(until !== null ? { until } : {}) };
}

// 데몬은 인자를 JSON 문자열로 남긴다 (잘렸을 수 있다): 배열이면 띄어 쓰고, 아니면 그대로
const argsText = (args) => {
    try {
        const v = JSON.parse(args);
        if (Array.isArray(v)) return v.map((x) => (typeof x === 'object' && x ? JSON.stringify(x) : x)).join(' ');
    } catch {}
    return args;
};

/** 한 건 → "무엇을" 한 줄 (명령이면 명령 이름과 인자, 원격 데몬에 중계한 조작이면 그 데몬과 요청) */
export function auditWhat(e, t) {
    const action = ACTIONS.includes(e.action) ? t(`audit.action_${e.action}`) : e.action;
    const detail = [e.command, argsText(e.args)].filter(Boolean).join(' ');
    return [action, e.daemon, detail].filter(Boolean).join(' · ');
}

/** 한 건 → 결과 칩. 예약 백업은 서버가 꺼져 있으면 409로 건너뛴다 */
export function auditResult(e, t) {
    if (e.ok) return { tone: 'ok', label: t('audit.ok') };
    if (e.action === 'backup' && e.status === 409) return { tone: 'idle', label: t('audit.skipped_stopped') };
    return { tone: 'stop', label: t('audit.failed', { status: e.status }) };
}

/** 한 건 → "누가·어디서" 한 줄 */
export function auditWho(e, t) {
    const source = SOURCES.includes(e.source) ? t(`audit.source_${e.source}`) : e.source;
    return [source, e.userName || e.user, e.guildName || e.guild].filter(Boolean).join(' · ');
}
