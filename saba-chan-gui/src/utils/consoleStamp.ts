// 콘솔 줄 앞 시각 (서버 콘솔과 데몬 로그가 같이 쓴다, 화면 없는 순수 함수)

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
