/** 콘솔 줄을 그리는 순수 계산 (앱 콘솔과 웹 콘솔이 같이 쓴다) */

export type ConsoleLevel = 'warn' | 'error';

/** 수준 필터에 걸리는 줄의 수준. 경고·오류가 아니면 null */
export const levelOf = (line: { level?: string | null }): ConsoleLevel | null => {
    const level = line.level?.toLowerCase();
    return level === 'error' ? 'error' : level === 'warn' || level === 'warning' ? 'warn' : null;
};

export type Stamp = ((ts: number) => string) | null;

/**
 * 줄 앞 시각 표시 함수. 끔이면 null. 시각은 데몬이 준 unix 초이고 이 PC 시간대·언어로 보인다
 */
export function consoleStamper(mode: string | undefined, withDate: boolean, lang: string | undefined): Stamp {
    if (!mode || mode === 'off') return null;
    const fmt = new Intl.DateTimeFormat(lang || undefined, {
        ...(withDate ? { month: '2-digit', day: '2-digit' } : {}),
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
        hour12: mode === '12h',
    });
    return (ts: number) => fmt.format(new Date(ts * 1000));
}

/**
 * 게임이 줄 맨 앞에 스스로 찍은 시각(모듈이 `timestamp` 토큰으로 선언한 조각)을 뺀 글자와 조각.
 * 데몬 시각을 줄 앞에 붙이는 화면에서 시각이 두 번 보이지 않게 한다. 그런 조각이 없거나,
 * 조각이 괄호를 열기만 하고 닫지 않아(`[12:00:00 INFO]`의 앞부분) 빼면 줄이 깨질 때는 그대로 둔다
 */
export function withoutOwnStamp(content: string, spans?: unknown): { content: string; spans?: unknown } {
    const first = Array.isArray(spans) ? spans[0] : null;
    const end = Array.isArray(first) && first[0] === 0 && first[2] === 'timestamp' ? Number(first[1]) || 0 : 0;
    const own = content.slice(0, end);
    if (!own || (own.match(/[[(]/g)?.length ?? 0) !== (own.match(/[\])]/g)?.length ?? 0)) return { content, spans };
    const rest = content.slice(end).trimStart();
    const cut = content.length - rest.length;
    return {
        content: rest,
        spans: (spans as unknown[]).slice(1).map((s) => (Array.isArray(s) ? [Math.max(0, s[0] - cut), s[1] - cut, s[2]] : s)),
    };
}
