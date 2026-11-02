import clsx from 'clsx';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { SabaArt } from '../ui/SabaArt';

/** 자는지 다시 묻는 간격 — 자동 코코낸내(착한 어린이)는 릴레이가 정한 시각에 잠들고 깬다 */
const POLL_MS = 60_000;
/** 깬 것을 본 뒤 일어나는 그림을 보이는 시간 */
const WAKING_MS = 10_000;

/**
 * 디스코드 서버 화면 구석의 작은 사바쨩: 코코낸내 중이면 자는 모습, 보는 동안 깨면 잠깐 일어나는 모습.
 * 자는지는 릴레이만 안다 (데몬 discord 설정 GET 의 `asleep`). 모르면 아무것도 보이지 않는다
 */
export function SleepCorner({ guildId }) {
    const { t } = useTranslation('gui');
    const [state, setState] = useState(null); // 'asleep' | 'waking' | null
    const asleep = useRef(null);

    useEffect(() => {
        if (!window.api?.discordSettingsGet) return undefined;
        let alive = true;
        let wakingTimer = null;
        asleep.current = null;
        setState(null);
        const check = async () => {
            let now = null;
            try {
                const res = await window.api.discordSettingsGet(guildId);
                if (typeof res?.asleep === 'boolean') now = res.asleep;
            } catch {
                // 못 읽으면 지난 모습을 그대로 둔다
                return;
            }
            if (!alive || now === null) return;
            const was = asleep.current;
            asleep.current = now;
            if (now) {
                clearTimeout(wakingTimer);
                setState('asleep');
            } else if (was) {
                setState('waking');
                wakingTimer = setTimeout(() => alive && setState(null), WAKING_MS);
            } else setState((s) => (s === 'waking' ? s : null));
        };
        check();
        const timer = setInterval(check, POLL_MS);
        return () => {
            alive = false;
            clearInterval(timer);
            clearTimeout(wakingTimer);
        };
    }, [guildId]);

    if (!state) return null;
    const label = t(state === 'asleep' ? 'discord.sleep_corner_asleep' : 'discord.sleep_corner_waking');
    return (
        <span className={clsx('dc-sleep-corner', state)}>
            <SabaArt kind={state === 'asleep' ? 'sleep' : 'sleepy'} size={52} alt={label} />
        </span>
    );
}
