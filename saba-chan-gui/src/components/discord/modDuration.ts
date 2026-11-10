import type { TFunction } from 'i18next';

/** 분 → "1시간 30분" 같은 말 (초는 seconds 로). 앱 서버 관리와 웹 디스코드 탭이 같은 문구(discord.mod.dur_*)를 쓴다 */
export function durationText(t: TFunction, { minutes = 0, seconds = 0 }: { minutes?: number; seconds?: number }) {
    const total = minutes * 60 + seconds;
    if (total < 60) return t('discord.mod.dur_seconds', { count: Math.round(total) });
    const m = Math.round(total / 60);
    const d = Math.floor(m / 1440);
    const h = Math.floor((m % 1440) / 60);
    const rest = m % 60;
    return [
        d && t('discord.mod.dur_days', { count: d }),
        h && t('discord.mod.dur_hours', { count: h }),
        rest && t('discord.mod.dur_minutes', { count: rest }),
    ]
        .filter(Boolean)
        .join(' ');
}

/** 슬로우 모드 고르기 (초) */
export const SLOW_SECONDS = [0, 5, 10, 30, 60, 300, 900, 3600, 21600];
/** 잠금 시간 고르기 (분) */
export const LOCK_MINUTES = [30, 60, 360, 1440, 10080];
/** 자동 청소 보관 시간 고르기 (분) */
export const AUTO_CLEAR_MINUTES = [60, 360, 720, 1440, 4320, 10080];
