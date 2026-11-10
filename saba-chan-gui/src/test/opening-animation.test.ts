import { describe, expect, it, vi } from 'vitest';
import {
    FALL_END_MS,
    fallFrame,
    fallPoint,
    OPENING_CHORD_MS,
    OPENING_END_MS,
    openingFrame,
    openingShake,
    pullPoint,
    scatterPoint,
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

    it('낱말은 모두 같은 규칙의 경로로 제자리에서 가운데로 끌려 온다 (경로에 무작위가 없다)', () => {
        const center = { x: 100, y: 100 };
        const from = { x: 160, y: 20 };
        expect(pullPoint(from, center, 0)).toEqual(from);
        expect(pullPoint(from, center, 1)).toEqual(center);
        const mid = pullPoint(from, center, 0.5);
        expect(Math.hypot(mid.x - 100, mid.y - 100)).toBeLessThan(100);
        expect(pullPoint(from, center, 0.5)).toEqual(mid);
    });

    it('터질 때는 낱말마다 정해 준 방향·거리로 흩뿌린다', () => {
        const center = { x: 100, y: 100 };
        expect(scatterPoint(center, 1, 200, 0)).toEqual(center);
        const end = scatterPoint(center, Math.PI / 2, 200, 1);
        expect(end.x).toBeCloseTo(100, 5);
        expect(end.y).toBeCloseTo(300, 5);
        const half = scatterPoint(center, Math.PI / 2, 200, 0.5);
        expect(half.y).toBeGreaterThan(200);
    });

    it('화음 순간 화면을 한 번 흔들고 곧 멈춘다', () => {
        expect(openingShake(OPENING_CHORD_MS - 1)).toBe(0);
        expect(openingShake(OPENING_CHORD_MS)).toBe(1);
        expect(openingShake(OPENING_CHORD_MS + 400)).toBeLessThan(0.02);
    });
});

describe('비정상 기동 (후두둑)', () => {
    it('섬광 없이 로딩 화면을 걷으며 메인 화면을 드러내고, 끝나면 마친다', () => {
        const start = fallFrame(0);
        expect(start.cover).toBe(1);
        let prev = 1;
        for (let ms = 0; ms < FALL_END_MS; ms += 50) {
            const f = fallFrame(ms);
            expect(f.cover).toBeLessThanOrEqual(prev);
            prev = f.cover;
            expect(f.done).toBe(false);
        }
        expect(fallFrame(FALL_END_MS)).toMatchObject({ cover: 0, done: true });
    });

    it('글자는 제자리에서 아래로 점점 빨리 떨어진다', () => {
        const from = { x: 40, y: 50 };
        expect(fallPoint(from, 0, 0, 600)).toEqual(from);
        const a = fallPoint(from, 200, 0, 600);
        const b = fallPoint(from, 400, 0, 600);
        expect(a.x).toBe(40);
        expect(a.y).toBeGreaterThan(50);
        expect(b.y - a.y).toBeGreaterThan(a.y - 50);
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
