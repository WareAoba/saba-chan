import { useState } from 'react';

/**
 * 디스코드 프로필 사진·서버 아이콘 (cdn.discordapp.com). 없거나 못 읽으면 이름 첫 글자.
 * `className`으로 크기를 정한다 (멤버 줄 `avatar`, 서버 칩 `guild-initial`, 작은 칩 `avatar avatar-sm`)
 */
export function Avatar({ url, name, className = 'avatar' }) {
    const [broken, setBroken] = useState(null);
    const ok = typeof url === 'string' && url.startsWith('https://') && broken !== url;
    return (
        <span className={className} aria-hidden="true">
            {ok ? (
                <img src={url} alt="" loading="lazy" referrerPolicy="no-referrer" onError={() => setBroken(url)} />
            ) : (
                String(name ?? '')
                    .trim()
                    .charAt(0)
                    .toUpperCase() || '?'
            )}
        </span>
    );
}
