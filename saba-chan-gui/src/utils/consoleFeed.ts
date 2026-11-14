import type { ApiBody } from '../types/daemon';

/**
 * 서버 콘솔 읽기 규칙 (앱 useMultiConsole과 웹 콘솔이 같이 쓴다. 화면 없는 순수 함수).
 * 콘솔 조회·스트림의 답 한 건과 지금까지 읽은 자리를 받아, 화면에 넣을 줄과 다음에 읽을 자리를 정한다.
 */

/** 콘솔 한 줄에서 이 규칙이 보는 것 */
export interface FeedLine {
    id?: string | number;
    timestamp?: number | null;
    source?: string;
    content?: string;
}

/** 지금까지 읽은 자리 */
export interface ConsoleFeed {
    /** 다음에 받을 줄 번호 */
    since: number;
    /** 마지막으로 본 콘솔 버퍼 세대(epoch). PID가 같아도 데몬이 다시 붙으면 바뀌고 줄 번호가 0부터 다시 시작한다 */
    epoch: number | null;
    /** 마지막으로 본 실행 PID. 바뀌면 새 프로세스라 줄 번호가 0부터 다시 시작한다 */
    pid: number | null;
    /** 세대가 바뀐 뒤 처음 받는 줄은 이어 붙이지 않고 그 버퍼로 바꾼다 (다시 붙은 버퍼는 이미 본 줄을 다시 담는다) */
    replaceNext: boolean;
    /** 꺼진 서버의 디스크 기록을 이미 받았다 (since와 상관없이 매번 같은 꼬리가 온다) */
    diskLoaded: boolean;
    /** 실행 중에 받은 마지막 몇 줄의 표시(lineMark). 꺼진 뒤 디스크 꼬리와 겹치는 곳을 찾는 데 쓴다 */
    seenTail: string[];
}

/** 앱과 웹이 아직 다르게 다루는 곳. 어느 쪽으로 맞출지 정하기 전까지 각자 지금대로 둔다 */
export interface FeedRules {
    /** 디스크 꼬리: true면 실행 중에 본 줄 뒤의 것만 잇고(앱), false면 화면을 그 꼬리로 바꾼다(웹) */
    diskUnseenOnly: boolean;
    /** 디스크 기록을 받은 뒤 세대가 바뀌면 PID가 그대로 보여도 새 실행으로 보고 잇는다(웹) */
    diskThenNewRun: boolean;
    /** 답에 stdin_available이 없으면(디스크 기록, 켠 적 없는 서버) 입력을 닫는다(웹). false면 앞의 값을 둔다(앱) */
    noStdinCloses: boolean;
    /** 세대를 주지 않는 데몬에서 PID가 바뀌면 처음부터 다시 받는다(앱) */
    pidResetsWithoutEpoch: boolean;
}

/** 답 한 건으로 정한 것 */
export interface ConsoleStep<L> {
    feed: ConsoleFeed;
    /** 조회가 옛 번호로 받았다: 받은 줄을 버리고 처음부터 다시 묻는다 (스트림은 데몬이 이미 처음부터 보낸다) */
    refetch: boolean;
    /** 화면에 넣을 줄. replace면 화면을 이것으로 바꾸고, 아니면 뒤에 잇는다 */
    lines: L[];
    replace: boolean;
    /** 입력 칸을 열지. undefined면 그대로 둔다 */
    stdin: boolean | undefined;
}

export const newConsoleFeed = (): ConsoleFeed => ({
    since: 0,
    epoch: null,
    pid: null,
    replaceNext: false,
    diskLoaded: false,
    seenTail: [],
});

/** 조회·스트림에 실어 보낼 자리 (데몬이 세대가 바뀌었는지, 디스크 기록을 다시 보낼지 정한다) */
export const feedCursor = (feed: ConsoleFeed) => ({
    since: feed.since,
    epoch: feed.epoch ?? undefined,
    diskSeen: feed.diskLoaded || undefined,
});

/** 지금 PID를 기억하고, 앞서 본 것과 다른 프로세스인지 */
export function notePid(feed: ConsoleFeed, pid: number | null | undefined, rules: FeedRules) {
    if (pid == null || pid <= 0) return { feed, changed: false };
    const changed = feed.pid !== null && pid !== feed.pid;
    // 세대를 주는 데몬이면 세대 비교가 정한다 (PID가 늦게 바뀌어도 줄이 겹치지 않게)
    const since = changed && feed.epoch === null && rules.pidResetsWithoutEpoch ? 0 : feed.since;
    return { feed: { ...feed, pid, since }, changed };
}

/** 디스크 줄과 실행 중에 받은 줄을 맞춰 보는 값. 디스크 기록은 내용 앞 공백을 지우고 시각은 초까지 남긴다 */
const lineMark = (line: FeedLine) => `${line.timestamp ?? ''}|${line.source ?? ''}|${String(line.content ?? '').trimStart()}`;

/**
 * 디스크 꼬리에서 이미 본 줄 뒤의 것만 (G-C3). 마지막으로 본 실행 줄들(seen)과 겹치는 곳을
 * 앞에서부터 찾는다: 같은 줄이 되풀이된 로그에서 모호하면 못 본 줄을 버리기보다 다시 보이는 쪽을 고른다.
 * 못 찾으면 다른 기록이라 보고 전부 준다
 */
function unseenDiskTail<L extends FeedLine>(seen: string[], tail: L[]) {
    if (seen.length === 0) return tail;
    const marks = tail.map(lineMark);
    for (let end = 0; end < tail.length; end++) {
        const span = Math.min(seen.length, end + 1);
        let match = true;
        for (let k = 0; k < span; k++) {
            if (marks[end - k] !== seen[seen.length - 1 - k]) {
                match = false;
                break;
            }
        }
        if (match) return tail.slice(end + 1);
    }
    return tail;
}

/**
 * 콘솔 조회·스트림의 답 한 건을 읽는다.
 * pidChanged: 이 답을 묻기 전에 notePid가 다른 프로세스를 봤다. fromStream: 스트림이 보낸 답이다
 */
export function consoleStep<L extends FeedLine>(
    feed: ConsoleFeed,
    res: ApiBody | null | undefined,
    { pidChanged, fromStream }: { pidChanged: boolean; fromStream: boolean },
    rules: FeedRules,
): ConsoleStep<L> {
    const next = { ...feed };
    const none = { feed: next, refetch: false, lines: [] as L[], replace: false, stdin: undefined };
    if (!Array.isArray(res?.lines)) return none;
    // epoch가 없는 응답(디스크 기록)은 지금처럼 이어 읽는다
    if (typeof res.epoch === 'number') {
        const epochChanged = (feed.epoch !== null && res.epoch !== feed.epoch) || res.reset === true;
        next.epoch = res.epoch;
        // 옛 번호로 받은 줄은 버리고 처음부터 다시 받는다. 새 프로세스면 이어 붙이고(지난 실행 기록을 남긴다),
        // 같은 프로세스에 다시 붙었으면 그 버퍼로 바꾼다
        if (epochChanged) {
            next.since = 0;
            next.replaceNext = !pidChanged && !(rules.diskThenNewRun && feed.diskLoaded);
            if (!fromStream) return { ...none, refetch: true };
        }
    }
    const stdin =
        typeof res.stdin_available === 'boolean' ? res.stdin_available : rules.noStdinCloses ? false : undefined;
    const got = res.lines as L[];
    // 꺼진 서버의 디스크 기록은 since와 상관없이 매번 같은 꼬리가 온다: 한 번만 넣고, 줄 번호(0부터)로 since를 올리지 않는다
    if (res.source === 'disk') {
        if (next.diskLoaded || !got.length) return { ...none, stdin };
        next.diskLoaded = true;
        if (!rules.diskUnseenOnly) return { ...none, stdin, lines: got, replace: true };
        next.seenTail = [];
        return { ...none, stdin, lines: unseenDiskTail(feed.seenTail, got) };
    }
    if (!got.length) return { ...none, stdin };
    // 프로세스가 다시 줄을 낸다: 다음에 꺼지면 디스크 기록을 다시 받는다
    next.diskLoaded = false;
    const replace = next.replaceNext;
    next.replaceNext = false;
    next.seenTail = [...(replace ? [] : feed.seenTail), ...got.map(lineMark)].slice(-5);
    next.since = Number(got[got.length - 1].id) + 1;
    return { ...none, stdin, lines: got, replace };
}
