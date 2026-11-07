// 지난 로그 한 줄의 시각을 이 PC 시간대로 (앱 지난 로그와 웹 로그 카드가 같이 쓴다)
//
// 데몬·러너는 로그 줄 머리에 UTC 시각을 적는다 (`[2026-11-07 16:40:57] [STDOUT/INFO] …`, logs.rs `line_time`).
// 게임이 찍는 시각은 PC 시간대라, 그대로 보이면 한 줄에 두 시각이 어긋난다. 화면에서만 바꾸고 파일·내보내기는 그대로 둔다.

const STAMP = /^\[(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2}):(\d{2})\]/;
const two = (n: number) => String(n).padStart(2, '0');

/** 줄 머리의 UTC 시각을 이 PC 시간대 `[YYYY-MM-DD HH:MM:SS]`로. 시각이 없는 줄은 그대로 */
export function localLogLine(line: string): string {
    const m = STAMP.exec(line);
    if (!m) return line;
    const [, y, mo, d, h, mi, s] = m.map(Number);
    const t = new Date(Date.UTC(y, mo - 1, d, h, mi, s));
    const local = `[${t.getFullYear()}-${two(t.getMonth() + 1)}-${two(t.getDate())} ${two(t.getHours())}:${two(t.getMinutes())}:${two(t.getSeconds())}]`;
    return local + line.slice(m[0].length);
}
