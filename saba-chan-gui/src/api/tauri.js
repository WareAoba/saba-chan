/**
 * window.__TAURI__ 전역(withGlobalTauri) 위의 얇은 감싸개. npm 의존 없이 invoke·listen만 쓴다.
 */
export function tauriBindings(tauri) {
    return {
        invoke: (cmd, args) => tauri.core.invoke(cmd, args),
        // 콜백에는 이벤트 본문(payload)만 넘긴다. 해제 함수의 Promise를 돌려준다
        listen: (name, cb) => tauri.event.listen(name, (event) => cb(event?.payload)),
    };
}
