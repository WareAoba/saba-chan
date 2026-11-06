import { create } from 'zustand';

/**
 * 서버별 콘솔 상태 (instanceId → { lines, input, ... }).
 * 콘솔은 0.5초마다 줄이 붙고 입력칸은 글자마다 바뀐다. App이 들고 있으면 앱 전체가 다시 그려지므로,
 * 여기 두고 그 콘솔을 그리는 곳만 구독한다. 쓰는 쪽은 useMultiConsole이다.
 */
/** 콘솔 한 칸 (useMultiConsole이 모양을 정한다) */
// biome-ignore lint/suspicious/noExplicitAny: 콘솔 칸 모양은 hooks/useMultiConsole이 정한다
export type ConsoleEntry = Record<string, any>;
export type Consoles = Record<string, ConsoleEntry>;

interface ConsoleState {
    consoles: Consoles;
    setConsoles: (consolesOrUpdater: Consoles | ((prev: Consoles) => Consoles)) => void;
}

export const useConsoleStore = create<ConsoleState>()((set) => ({
    consoles: {},
    setConsoles: (consolesOrUpdater) =>
        set((state) => ({
            consoles: typeof consolesOrUpdater === 'function' ? consolesOrUpdater(state.consoles) : consolesOrUpdater,
        })),
}));
