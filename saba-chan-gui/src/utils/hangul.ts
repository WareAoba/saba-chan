/** 한글 검색 도우미: 초성 검색 ("ㅂㅇ"으로 "백업"을 찾는다) */

// 초성 19자 (호환 자모 ㄱ ㄲ ㄴ ㄷ ㄸ ㄹ ㅁ ㅂ ㅃ ㅅ ㅆ ㅇ ㅈ ㅉ ㅊ ㅋ ㅌ ㅍ ㅎ)
const CHOSEONG = '\u3131\u3132\u3134\u3137\u3138\u3139\u3141\u3142\u3143\u3145\u3146\u3147\u3148\u3149\u314a\u314b\u314c\u314d\u314e';
const HANGUL_START = 0xac00;
const HANGUL_COUNT = 11172;

/** 완성형 한글은 초성으로, 나머지 글자는 그대로 */
export function choseong(text: string) {
    return Array.from(text)
        .map((c) => {
            const k = c.charCodeAt(0) - HANGUL_START;
            return k >= 0 && k < HANGUL_COUNT ? CHOSEONG[Math.floor(k / 588)] : c;
        })
        .join('');
}

const onlyChoseong = (word: string) => /^[\u3131-\u314e]+$/.test(word);

/**
 * 검색 점수: 빈칸으로 나눈 조각이 모두 들어 있어야 맞는다(0이면 안 맞음). 조각마다 앞에서 맞으면 3, 가운데면 2,
 * 초성만 친 조각이 초성으로 맞으면 1. 빈 검색은 1.
 */
export function matchScore(haystack: string, query: string) {
    const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
    if (!words.length) return 1;
    const hay = haystack.toLowerCase();
    const hayChoseong = choseong(hay);
    let score = 0;
    for (const w of words) {
        if (hay.includes(w)) score += hay.startsWith(w) ? 3 : 2;
        else if (onlyChoseong(w) && hayChoseong.includes(w)) score += 1;
        else return 0;
    }
    return score;
}
