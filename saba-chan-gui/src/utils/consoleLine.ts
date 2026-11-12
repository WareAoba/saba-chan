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
