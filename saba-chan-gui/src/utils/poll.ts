/**
 * `fn`을 `ms`마다 부른다. 앞 요청이 끝난 뒤에야 다음을 잡아서 느린 연결에 요청이 쌓이지 않고,
 * 창·탭이 숨어 있는 동안은 쉬었다가 다시 보이면 바로 읽는다. 멈추는 함수를 돌려준다.
 * `immediate: false`면 처음 읽기를 `ms` 뒤로 미룬다. `ms`가 함수면 다음 읽기를 잡을 때마다 묻는다 (앱·웹 공용)
 */
export function startPoll(
    fn: () => unknown,
    ms: number | (() => number),
    { immediate = true }: { immediate?: boolean } = {},
) {
    const gap = typeof ms === 'function' ? ms : () => ms;
    let stopped = false;
    let running = false;
    let timer: ReturnType<typeof setTimeout> | null = null;
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
        if (!stopped && !document.hidden) timer = setTimeout(tick, gap());
    };
    const onVisible = () => {
        if (document.hidden || timer || running) return;
        tick();
    };
    document.addEventListener('visibilitychange', onVisible);
    if (immediate) tick();
    else if (!document.hidden) timer = setTimeout(tick, gap());
    return () => {
        stopped = true;
        if (timer) clearTimeout(timer);
        document.removeEventListener('visibilitychange', onVisible);
    };
}
