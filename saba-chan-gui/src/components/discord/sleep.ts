/** 코코낸내 값 (화면 없는 상수, GUI 봇 설정과 웹 디스코드 탭이 같이 쓴다) */

/** 코코낸내 동안 끌 수 있는 기능 묶음 (데몬 discord.rs `SLEEP_FEATURES`) */
export const SLEEP_FEATURES: string[] = ['commands', 'llm', 'music', 'tts', 'extras'];
/** 자동 코코낸내를 켤 때 처음 시각 */
export const DEFAULT_BEDTIME = { start: '01:00', end: '08:00' };
/** 고를 수 있는 시각 — 30분 단위 (CLI 로 다른 시각을 저장했으면 그 시각도 보인다) */
export const HALF_HOURS = Array.from(
    { length: 48 },
    (_, i) => `${String(Math.floor(i / 2)).padStart(2, '0')}:${i % 2 ? '30' : '00'}`,
);
