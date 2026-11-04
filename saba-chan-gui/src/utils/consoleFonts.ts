/**
 * 콘솔 글꼴: 글자(영문·한글·일본어·중국어)마다 하나씩 고른다. 앱·웹이 같은 목록과 같은 글꼴 파일을 쓴다
 *
 * 글꼴 파일은 styles/consoleFonts.css 가 `saba-<글자>-<글꼴>` 이름과 글자 범위(unicode-range)로 싣는다.
 * 한 줄에 여러 언어가 섞여도 글자마다 그 글자의 글꼴로 그린다. 고른 글꼴만 내려받는다.
 * 고를 수 있는 값은 데몬(src/config_store ConsoleFonts)이 같은 목록으로 검사한다.
 */

/** 글꼴 이름 (고유 명사라 번역하지 않는다) */
export const CONSOLE_FONT_NAMES = {
    'cascadia-code': 'Cascadia Code',
    d2coding: 'D2Coding',
    'nanum-gothic-coding': '나눔고딕코딩',
    'neodgm-code': '네오둥근모 코드',
    'sarasa-mono-j': 'Sarasa Mono J',
    'udev-gothic': 'UDEV Gothic',
    hackgen: 'HackGen',
    'm-plus-1-code': 'M PLUS 1 Code',
    'sarasa-mono-sc': 'Sarasa Mono SC',
    'sarasa-mono-tc': 'Sarasa Mono TC',
};

const KO = ['d2coding', 'nanum-gothic-coding', 'neodgm-code'];
const JA = ['sarasa-mono-j', 'udev-gothic', 'hackgen', 'm-plus-1-code'];
const ZH = ['sarasa-mono-sc', 'sarasa-mono-tc'];

/** 글자마다 고를 수 있는 글꼴. 영문은 모든 후보 */
export const CONSOLE_FONT_CHOICES = {
    latin: ['cascadia-code', ...KO, ...JA, ...ZH],
    ko: KO,
    ja: JA,
    zh: ZH,
};

/** 미리보기 견본: 한 줄에 영문·한글·일본어·한자 (번역하지 않는다 — 글자마다 다른 글꼴로 그려지는 것을 보인다) */
export const CONSOLE_FONT_SAMPLE = '| 접속 3/32 | 接続 3/32 | 在线 3/32';

export type ConsoleFontSlot = keyof typeof CONSOLE_FONT_CHOICES;

export const CONSOLE_FONT_SLOTS = Object.keys(CONSOLE_FONT_CHOICES) as ConsoleFontSlot[];

export const CONSOLE_FONT_DEFAULTS = Object.freeze({
    latin: 'cascadia-code',
    ko: 'd2coding',
    ja: 'hackgen',
    zh: 'sarasa-mono-sc',
});

/** 고른 값을 칸마다 확인한다. 없거나 그 글자의 후보가 아니면 기본값 */
export function normalizeConsoleFonts(fonts?: Partial<Record<ConsoleFontSlot, string>> | null) {
    const out = {} as Record<ConsoleFontSlot, string>;
    for (const slot of CONSOLE_FONT_SLOTS) {
        const v = fonts?.[slot] as string;
        out[slot] = CONSOLE_FONT_CHOICES[slot].includes(v) ? v : CONSOLE_FONT_DEFAULTS[slot];
    }
    return out;
}

/** CSS font-family 값. 한자는 화면 언어가 중국어면 중국어 글꼴, 그 밖에는 일본어 글꼴을 먼저 쓴다 */
export function consoleFontStack(fonts?: Partial<Record<ConsoleFontSlot, string>> | null, language = '') {
    const f = normalizeConsoleFonts(fonts);
    const order: ConsoleFontSlot[] = String(language).toLowerCase().startsWith('zh')
        ? ['latin', 'ko', 'zh', 'ja']
        : ['latin', 'ko', 'ja', 'zh'];
    return [...order.map((slot) => `'saba-${slot}-${f[slot]}'`), 'monospace'].join(', ');
}

/** 화면의 고정폭 글꼴(--font-mono)을 고른 글꼴로 */
export function applyConsoleFonts(fonts?: Partial<Record<ConsoleFontSlot, string>> | null, language?: string) {
    document.documentElement.style.setProperty('--font-mono', consoleFontStack(fonts, language));
}
