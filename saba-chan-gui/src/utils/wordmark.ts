/** 공식 글자 로고: 한국어·일본어·영어는 각자, 나머지 언어는 영어. `srcs`는 앱(public 경로)과 웹(빌드 자산)이 따로 준다. ratio = 가로/세로 */
export function wordmarkFor<T>(lang: string | null | undefined, srcs: { ko: T; ja: T; en: T }) {
    const l = String(lang || 'en').toLowerCase();
    if (l.startsWith('ko')) return { src: srcs.ko, ratio: 996 / 300 };
    if (l.startsWith('ja')) return { src: srcs.ja, ratio: 1459 / 300 };
    return { src: srcs.en, ratio: 2236 / 300 };
}
