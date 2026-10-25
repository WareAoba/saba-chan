// 그림은 GUI(saba-chan-gui/public)의 것을 그대로 쓴다. Vite가 빌드 때 assets/로 옮긴다
import favicon from '@art/favicon.png';
import icon from '@art/icon.png';
import logoEn from '@art/logo-en.png';
import logoJp from '@art/logo-jp.png';
import logoKr from '@art/logo-kr.png';
import relay from '@art/saba/relay.webp';
import sleep from '@art/saba/sleep.webp';

import denied from './assets/denied-900.webp';

export const art = { favicon, icon, relay, sleep, denied };

/** 공식 글자 로고 (GUI AppBar 와 같은 규칙: 한국어·일본어·영어, 나머지는 영어). ratio = 가로/세로 */
export function wordmarkFor(lang) {
    const l = String(lang || 'en').toLowerCase();
    if (l.startsWith('ko')) return { src: logoKr, ratio: 996 / 300 };
    if (l.startsWith('ja')) return { src: logoJp, ratio: 1459 / 300 };
    return { src: logoEn, ratio: 2236 / 300 };
}
