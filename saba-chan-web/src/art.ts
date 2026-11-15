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

/** 공식 글자 로고 그림 (고르는 규칙은 @gui/utils/wordmark) */
export const wordmarks = { ko: logoKr, ja: logoJp, en: logoEn };
