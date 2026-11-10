import { describe, expect, it, vi } from 'vitest';
import {
    beginOpeningSound,
    CHIME_CHORD_MS,
    claimOpeningSound,
    FALL_END_MS,
    fallFrame,
    fallPoint,
    OPENING_CHIME_LEAD_MS,
    OPENING_CHORD_MS,
    OPENING_END_MS,
    openingChordMs,
    openingFrame,
    openingShake,
    pullPoint,
    scatterPoint,
    takeOpeningSound,
    whenOpeningReady,
    wordPull,
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

    it('기동음을 오프닝보다 조금 늦게 틀어, 그 화음이 오프닝의 화음 순간에 맞는다', () => {
        expect(OPENING_CHIME_LEAD_MS).toBeGreaterThan(0);
        expect(OPENING_CHORD_MS).toBe(OPENING_CHIME_LEAD_MS + CHIME_CHORD_MS);
    });

    it('낱말은 정한 차례대로 하나씩 떨어져 나와, 마지막 낱말이 화음에 가운데 닿는다', () => {
        // 첫 낱말은 곧바로 떠나 화음 전에 닿는다
        expect(wordPull(0, 0)).toBe(0);
        expect(wordPull(OPENING_CHORD_MS * 0.2, 0)).toBeGreaterThan(0);
        expect(wordPull(OPENING_CHORD_MS * 0.7, 0)).toBe(1);
        // 마지막 낱말은 한참 제자리에 있다가 화음에 닿는다
        expect(wordPull(OPENING_CHORD_MS * 0.4, 1)).toBe(0);
        expect(wordPull(OPENING_CHORD_MS - 1, 1)).toBeLessThan(1);
        expect(wordPull(OPENING_CHORD_MS, 1)).toBe(1);
        // 어느 때든 먼저 떠난 낱말이 더 많이 와 있고, 낱말마다 끊김 없이 다가온다
        for (let ms = 0; ms <= OPENING_CHORD_MS; ms += 20) {
            expect(wordPull(ms, 0.3)).toBeGreaterThanOrEqual(wordPull(ms, 0.6));
            expect(wordPull(ms + 20, 0.5)).toBeGreaterThanOrEqual(wordPull(ms, 0.5));
        }
    });

    it('로고는 작게 쪼그라들지 않고 제 크기로 있다가 화음 무렵에 옅어진다', () => {
        expect(openingFrame(OPENING_CHORD_MS * 0.5).content).toBe(1);
        expect(openingFrame(OPENING_CHORD_MS - 1).content).toBeLessThan(0.2);
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

describe('오프닝 소리 (처음 사용자는 가이드 열림음에 맞춘다)', () => {
    it('가이드 열림음은 기동음보다 화음이 늦어, 그만큼 오프닝을 길게 끌어온다', () => {
        const chord = openingChordMs('guideOpen');
        expect(chord).toBeGreaterThan(openingChordMs('startup'));
        expect(openingChordMs('startup')).toBe(OPENING_CHORD_MS);
        expect(openingFrame(OPENING_CHORD_MS + 10, chord).opened).toBe(false);
        expect(openingFrame(chord, chord).opened).toBe(true);
        expect(wordPull(chord - 1, 1, chord)).toBeLessThan(1);
        expect(wordPull(chord, 1, chord)).toBe(1);
        expect(openingShake(chord, chord)).toBe(1);
    });

    it('오프닝을 준비하는 동안 열린 화면이 소리를 맡기면 오프닝이 대신 낸다', () => {
        expect(claimOpeningSound('guideOpen')).toBe(false);
        beginOpeningSound();
        expect(claimOpeningSound('guideOpen')).toBe(true);
        expect(takeOpeningSound()).toBe('guideOpen');
        // 오프닝이 소리를 가져간 뒤에는 화면이 직접 낸다
        expect(claimOpeningSound('guideOpen')).toBe(false);
        beginOpeningSound();
        expect(takeOpeningSound()).toBeNull();
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
