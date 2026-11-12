import { create } from 'zustand';

/**
 * 서버별 콘솔 상태 (instanceId → { lines, input, ... }).
 * 콘솔은 0.5초마다 줄이 붙고 입력칸은 글자마다 바뀐다. App이 들고 있으면 앱 전체가 다시 그려지므로,
 * 여기 두고 그 콘솔을 그리는 곳만 구독한다. 쓰는 쪽은 useMultiConsole이다.
 */
/** 콘솔 한 줄 */
export interface ConsoleLine {
    /** GUI가 매기는 그리기용 key (데몬 id는 겹칠 수 있다) */
    key?: string | number;
    id?: string | number;
    source?: string;
    level?: string;
    timestamp?: number | null;
    content: string;
    spans?: unknown[];
    /** 보낸 명령 줄(source: input)을 시킨 쪽 */
    by?: { source?: string; userName?: string };
}

/** 콘솔 한 칸 */
export interface ConsoleEntry {
    server: { id: string; name: string };
    lines: ConsoleLine[];
    input: string;
    stdinDisabled: boolean;
}
export type Consoles = Record<string, ConsoleEntry>;

interface ConsoleState {
    consoles: Consoles;
    /** PIP로 띄울 때 남겨 둔 마지막 줄 글자 (메인 창 콘솔 자리에 흐리게 깐다) */
    ghosts: Record<string, string[]>;
    setConsoles: (consolesOrUpdater: Consoles | ((prev: Consoles) => Consoles)) => void;
}

export const useConsoleStore = create<ConsoleState>()((set) => ({
    consoles: {},
    ghosts: {},
    setConsoles: (consolesOrUpdater) =>
        set((state) => ({
            consoles: typeof consolesOrUpdater === 'function' ? consolesOrUpdater(state.consoles) : consolesOrUpdater,
        })),
}));
