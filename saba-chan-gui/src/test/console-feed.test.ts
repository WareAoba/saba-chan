import { describe, expect, it } from 'vitest';
import { type ConsoleFeed, consoleStep, type FeedRules, newConsoleFeed, notePid } from '../utils/consoleFeed';

const APP: FeedRules = { diskUnseenOnly: true, diskThenNewRun: false, noStdinCloses: false, pidResetsWithoutEpoch: true };
const WEB: FeedRules = { diskUnseenOnly: false, diskThenNewRun: true, noStdinCloses: true, pidResetsWithoutEpoch: false };
const run = (contents: string[], from = 0) => contents.map((content, i) => ({ id: from + i, content }));
const step = (feed: ConsoleFeed, res: object, rules = APP, how = { pidChanged: false, fromStream: false }) =>
    consoleStep(feed, res, how, rules);

describe('콘솔 읽기 규칙', () => {
    it('조회가 다른 세대를 받으면 줄을 버리고 처음부터 다시 묻고, 같은 프로세스면 그 버퍼로 바꾼다', () => {
        let feed = step(newConsoleFeed(), { epoch: 1, lines: run(['a', 'b']) }).feed;
        expect(feed.since).toBe(2);

        const stale = step(feed, { epoch: 2, lines: run(['x']) });
        expect(stale).toMatchObject({ refetch: true, lines: [] });
        expect(stale.feed.since).toBe(0);

        const again = step(stale.feed, { epoch: 2, lines: run(['x', 'y']) });
        expect(again).toMatchObject({ replace: true, lines: run(['x', 'y']) });
        expect(again.feed.since).toBe(2);
    });

    it('스트림의 reset은 다시 묻지 않고 받은 줄을 넣는다. 새 프로세스면 지난 줄 뒤에 잇는다', () => {
        const feed = step(newConsoleFeed(), { epoch: 1, lines: run(['a']) }).feed;
        const streamed = step(feed, { epoch: 2, reset: true, lines: run(['new']) }, APP, { pidChanged: true, fromStream: true });
        expect(streamed).toMatchObject({ refetch: false, replace: false, lines: run(['new']) });
    });

    it('꺼진 서버의 디스크 기록은 한 번만 넣고 줄 번호를 올리지 않는다', () => {
        let feed = step(newConsoleFeed(), { epoch: 1, lines: run(['boot', 'tick']) }).feed;
        const disk = { source: 'disk', lines: run(['boot', 'tick', 'panic!']) };

        const app = step(feed, disk);
        expect(app).toMatchObject({ replace: false, lines: [{ content: 'panic!' }] });
        expect(app.feed.since).toBe(2);
        expect(step(app.feed, disk).lines).toEqual([]);

        // 웹은 화면을 디스크 꼬리로 바꾼다
        feed = step(feed, disk, WEB).feed;
        expect(step(newConsoleFeed(), disk, WEB)).toMatchObject({ replace: true, lines: disk.lines });
        expect(step(feed, disk, WEB).lines).toEqual([]);
    });

    it('앱과 웹이 다른 곳: 디스크 뒤 세대 바뀜, stdin을 모를 때, 세대 없는 데몬의 PID 바뀜', () => {
        const disk = step(step(newConsoleFeed(), { epoch: 1, lines: run(['a']) }).feed, { source: 'disk', lines: run(['a', 'b']) }).feed;
        const next = { epoch: 2, reset: true, lines: run(['c']) };
        const how = { pidChanged: false, fromStream: true };
        expect(step(disk, next, APP, how).replace).toBe(true);
        expect(step(disk, next, WEB, how).replace).toBe(false);

        expect(step(newConsoleFeed(), { source: 'none', lines: [] }, APP).stdin).toBeUndefined();
        expect(step(newConsoleFeed(), { source: 'none', lines: [] }, WEB).stdin).toBe(false);

        const old = { ...newConsoleFeed(), pid: 10, since: 5 };
        expect(notePid(old, 20, APP)).toMatchObject({ changed: true, feed: { pid: 20, since: 0 } });
        expect(notePid(old, 20, WEB)).toMatchObject({ changed: true, feed: { pid: 20, since: 5 } });
        expect(notePid(old, null, APP)).toMatchObject({ changed: false, feed: { pid: 10 } });
    });
});
