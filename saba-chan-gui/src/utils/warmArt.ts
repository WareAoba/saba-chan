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
    const idle = (window as Window & { requestIdleCallback?: (cb: () => void, o?: { timeout: number }) => number })
        .requestIdleCallback;
    if (idle) idle(run, { timeout: 3000 });
    else setTimeout(run, 1000);
}
