import { create } from 'zustand';

/**
 * 서버별 콘솔 상태 (instanceId → { lines, input, ... }).
 * 콘솔은 0.5초마다 줄이 붙고 입력칸은 글자마다 바뀐다. App이 들고 있으면 앱 전체가 다시 그려지므로,
 * 여기 두고 그 콘솔을 그리는 곳만 구독한다. 쓰는 쪽은 useMultiConsole이다.
 */
export const useConsoleStore = create((set) => ({
    consoles: {},
    setConsoles: (consolesOrUpdater) =>
        set((state) => ({
            consoles: typeof consolesOrUpdater === 'function' ? consolesOrUpdater(state.consoles) : consolesOrUpdater,
        })),
}));
