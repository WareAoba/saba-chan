/**
 * `ms`마다 `fn`을 부르되 창이 숨어 있으면(최소화·트레이) 건너뛰고, 다시 보이면 바로 한 번 부른다.
 * 처음 한 번은 부르지 않는다 (부르는 쪽이 먼저 부른다). 멈추는 함수를 돌려준다
 */
export function everyWhileVisible(fn: () => void, ms: number): () => void {
    const timer = setInterval(() => {
        if (!document.hidden) fn();
    }, ms);
    const onVisible = () => {
        if (!document.hidden) fn();
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => {
        clearInterval(timer);
        document.removeEventListener('visibilitychange', onVisible);
    };
}
