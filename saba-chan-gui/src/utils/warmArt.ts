/**
 * 탭마다 처음 열 때 받는 그림·글꼴을, 오프닝이 끝나고 한가할 때 미리 받아 풀어 둔다.
 * 앱 안 파일도 첫 요청은 100ms 넘게 걸려(11-12 PC 측정) 디스코드·정보 탭을 처음 열면 그림이 늦게 뜬다
 */
const ART = [
    './saba/relay.webp',
    './server-chan.webp',
    './title.png',
    './saba/mermaid-upper.webp',
    './saba/cooking.webp',
    './saba/recovering.webp',
    './saba/sleep.webp',
];

export function warmTabArt() {
    const run = () => {
        for (const src of ART) {
            const img = new Image();
            img.decoding = 'async';
            img.src = src;
            img.decode?.().catch(() => {});
        }
        document.fonts?.load?.("700 1em 'saba-hand'").catch(() => {});
    };
    whenIdle(run);
}

type IdleWindow = Window & {
    requestIdleCallback?: (cb: () => void, o?: { timeout: number }) => number;
    cancelIdleCallback?: (id: number) => void;
};

/** 한가할 때(늦어도 3초 안에) 한 번 부른다. 없는 웹뷰(맥 WebKit)는 1초 뒤. 돌려준 함수로 취소한다 */
export function whenIdle(run: () => void): () => void {
    const w = window as IdleWindow;
    if (w.requestIdleCallback) {
        const id = w.requestIdleCallback(run, { timeout: 3000 });
        return () => w.cancelIdleCallback?.(id);
    }
    const timer = setTimeout(run, 1000);
    return () => clearTimeout(timer);
}
