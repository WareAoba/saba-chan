/**
 * `fn`을 `ms`마다 부른다. 앞 요청이 끝난 뒤에야 다음을 잡아서 느린 연결에 요청이 쌓이지 않고,
 * 탭이 숨어 있는 동안은 쉬었다가 다시 보이면 바로 읽는다. 멈추는 함수를 돌려준다
 */
export function startPoll(fn, ms) {
    let stopped = false;
    let running = false;
    let timer = null;
    const tick = async () => {
        timer = null;
        if (stopped || running || document.hidden) return;
        running = true;
        try {
            await fn();
        } catch {
            // 한 번 실패해도 다음 주기에 다시 읽는다
        } finally {
            running = false;
        }
        if (!stopped && !document.hidden) timer = setTimeout(tick, ms);
    };
    const onVisible = () => {
        if (document.hidden || timer || running) return;
        tick();
    };
    document.addEventListener('visibilitychange', onVisible);
    tick();
    return () => {
        stopped = true;
        if (timer) clearTimeout(timer);
        document.removeEventListener('visibilitychange', onVisible);
    };
}
