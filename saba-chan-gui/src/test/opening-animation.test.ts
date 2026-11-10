import { describe, expect, it, vi } from 'vitest';
import {
    burstPoint,
    OPENING_CHORD_MS,
    OPENING_END_MS,
    openingFrame,
    pullPoint,
    whenOpeningReady,
} from '../utils/opening';

describe('오프닝 애니메이션 (콘솔 글자를 끌어모아 화음에 팟)', () => {
    it('화음 전에는 메인 화면을 가리고, 글자를 처음부터 화음까지 끊김 없이 끌어온다', () => {
        let prev = -1;
        for (let ms = 0; ms < OPENING_CHORD_MS; ms += 20) {
            const f = openingFrame(ms);
            expect(f.opened).toBe(false);
            expect(f.flash).toBe(0);
            expect(f.pull).toBeGreaterThanOrEqual(prev);
            prev = f.pull;
        }
        expect(openingFrame(0).pull).toBe(0);
        expect(openingFrame(OPENING_CHORD_MS / 2).pull).toBeGreaterThan(0);
        expect(openingFrame(OPENING_CHORD_MS - 1).pull).toBeLessThan(1);
    });

    it('마지막 음(화음)에 글자가 가운데 닿고, 그 순간 섬광과 함께 메인 화면이 열린다', () => {
        const f = openingFrame(OPENING_CHORD_MS);
        expect(f.pull).toBe(1);
        expect(f.opened).toBe(true);
        expect(f.flash).toBeGreaterThan(0.8);
        expect(f.burst).toBe(0);
        const later = openingFrame(OPENING_CHORD_MS + 300);
        expect(later.flash).toBeLessThan(0.2);
        expect(later.burst).toBeGreaterThan(0);
    });

    it('흩어짐이 끝나면 오프닝을 마친다', () => {
        expect(openingFrame(OPENING_END_MS - 1).done).toBe(false);
        expect(openingFrame(OPENING_END_MS).done).toBe(true);
    });

    it('글자는 제자리에서 가운데로 끌려 오고, 터질 때는 온 방향으로 다시 튀어 나간다', () => {
        const center = { x: 100, y: 100 };
        const from = { x: 160, y: 20 };
        expect(pullPoint(from, center, 0)).toEqual(from);
        expect(pullPoint(from, center, 1)).toEqual(center);
        const mid = pullPoint(from, center, 0.5);
        expect(Math.hypot(mid.x - 100, mid.y - 100)).toBeLessThan(100);

        expect(burstPoint(from, center, 0)).toEqual(center);
        const out = burstPoint(from, center, 1);
        const dir = Math.atan2(from.y - 100, from.x - 100);
        expect(Math.atan2(out.y - 100, out.x - 100)).toBeCloseTo(dir, 5);
        expect(Math.hypot(out.x - 100, out.y - 100)).toBeGreaterThan(100);
    });
});

describe('오프닝 준비 (메인 화면을 다 그린 뒤 재생)', () => {
    const frames = () => {
        const queue: (() => void)[] = [];
        return {
            next: (cb: () => void) => {
                queue.push(cb);
            },
            flush: () => {
                for (const cb of queue.splice(0)) cb();
            },
        };
    };

    it('기다릴 것이 모두 끝나고 화면이 두 번 그려진 뒤에야 재생한다', async () => {
        const f = frames();
        let finish!: () => void;
        const fonts = new Promise<void>((r) => {
            finish = r;
        });
        const ready = vi.fn();
        whenOpeningReady([fonts], 5000, f.next).then(ready);
        await Promise.resolve();
        f.flush();
        await Promise.resolve();
        expect(ready).not.toHaveBeenCalled();
        finish();
        await new Promise((r) => setTimeout(r, 0));
        f.flush();
        await Promise.resolve();
        expect(ready).not.toHaveBeenCalled();
        f.flush();
        await new Promise((r) => setTimeout(r, 0));
        expect(ready).toHaveBeenCalledTimes(1);
    });

    it('끝나지 않는 준비가 있어도 정한 시간이 지나면 재생한다', async () => {
        vi.useFakeTimers();
        try {
            const ready = vi.fn();
            const next = (cb: () => void) => setTimeout(cb, 16);
            whenOpeningReady([new Promise(() => {})], 1500, next).then(ready);
            await vi.advanceTimersByTimeAsync(1400);
            expect(ready).not.toHaveBeenCalled();
            await vi.advanceTimersByTimeAsync(200);
            expect(ready).toHaveBeenCalledTimes(1);
        } finally {
            vi.useRealTimers();
        }
    });

    it('준비 하나가 실패해도 재생한다', async () => {
        const ready = vi.fn();
        await whenOpeningReady([Promise.reject(new Error('x'))], 1500, (cb) => setTimeout(cb, 0)).then(ready);
        expect(ready).toHaveBeenCalledTimes(1);
    });
});
