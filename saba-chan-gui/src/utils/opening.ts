/**
 * 오프닝(로딩 화면이 걷히는 순간)과 정상 기동음.
 *
 * 기동음은 데몬과 릴레이가 둘 다 정상으로 붙었을 때 오프닝과 함께 한 번 낸다. 릴레이 첫 확인은 데몬 준비 뒤에
 * 오므로, 그 답을 받을 때까지(최대 RELAY_WAIT_MS) 오프닝을 붙잡아 소리와 애니메이션이 같이 시작하게 한다.
 * 릴레이를 아직 연결하지 않았으면 실패한 것이 없으니 데몬만으로 낸다.
 */
export const RELAY_WAIT_MS = 4000;

export interface OpeningInput {
    /** 데몬에 붙어서 연 것인가 (데몬 없이 '그대로 열기'는 false) */
    daemonOk: boolean;
    /** 봇 설정(릴레이 호스트)을 읽었는가 */
    configLoaded: boolean;
    relayConfigured: boolean;
    /** 릴레이 첫 확인이 답했는가 */
    relayChecked: boolean;
    relayConnected: boolean;
    /** 데몬 준비 뒤 릴레이를 기다린 시간 */
    waitedMs: number;
}

export function openingGate(s: OpeningInput): { hold: boolean; chime: boolean } {
    if (!s.daemonOk) return { hold: false, chime: false };
    const relayUnknown = !s.configLoaded || (s.relayConfigured && !s.relayChecked);
    if (relayUnknown && s.waitedMs < RELAY_WAIT_MS) return { hold: true, chime: false };
    if (relayUnknown) return { hold: false, chime: false };
    return { hold: false, chime: !s.relayConfigured || s.relayConnected };
}

/*
 * 오프닝 애니메이션. 메인 화면은 재생 전에 로딩 화면 아래에 다 그려 둔다.
 * - 정상 기동(burst, 기동음과 함께): 로딩 화면에 실제로 떠 있던 콘솔 낱말을 모두 같은 규칙의 곡선으로 가운데로 쭉
 *   끌어모으고(낱말마다 회전만 무작위), 가운데에서 먼지로 부서져 합쳐진다. 기동음의 마지막 음(화음)에 섬광·충격파·
 *   화면 흔들림과 함께 메인 화면을 열고 낱말을 무작위 방향·거리로 흩뿌린다.
 * - 비정상 기동(fall, 소리 없음): 낱말이 제자리에서 후두둑 떨어지고 로고가 가라앉으며 메인 화면이 드러난다.
 */
export type OpeningKind = 'burst' | 'fall';

/** 기동음의 마지막 음(도·라·도 화음)이 울리는 때. 248 BPM 네 번째 박 (app-startup.wav) */
export const OPENING_CHORD_MS = 726;
/** 메인 화면 준비(글꼴·그림)를 이보다 오래 기다리지 않는다 */
export const OPENING_READY_WAIT_MS = 1500;
/** 화음 뒤 글자가 흩어지는 길이 */
export const OPENING_BURST_MS = 700;
export const OPENING_END_MS = OPENING_CHORD_MS + OPENING_BURST_MS;
/** 섬광이 걷히는 빠르기 (지수 감쇠 시간 상수) */
const FLASH_DECAY_MS = 120;

export interface OpeningFrame {
    /** 끌어오기 진행 0..1 (화음에서 1) */
    pull: number;
    /** 흩어지기 진행 0..1 (화음에서 0) */
    burst: number;
    /** 메인 화면을 열었는가 (화음부터) */
    opened: boolean;
    /** 섬광 세기 0..1 */
    flash: number;
    /** 가운데 묶음(로고·글자 로고·진행 줄)의 크기와 불투명도 */
    content: number;
    done: boolean;
}

const clamp01 = (x: number) => Math.min(1, Math.max(0, x));

export function openingFrame(ms: number): OpeningFrame {
    if (ms < OPENING_CHORD_MS) {
        const p = clamp01(ms / OPENING_CHORD_MS);
        // 처음엔 천천히, 화음에 가까울수록 빨라진다 (박자에 끊지 않고 한 번에 쭉)
        return { pull: p * p * p, burst: 0, opened: false, flash: 0, content: 1 - clamp01(p * 1.6), done: false };
    }
    const after = ms - OPENING_CHORD_MS;
    return {
        pull: 1,
        burst: clamp01(after / OPENING_BURST_MS),
        opened: true,
        flash: Math.exp(-after / FLASH_DECAY_MS),
        content: 0,
        done: ms >= OPENING_END_MS,
    };
}

interface Point {
    x: number;
    y: number;
}

/** 끌려 오는 글자의 자리: 가운데를 조금 감아 돌며 들어온다 */
export function pullPoint(from: Point, center: Point, pull: number): Point {
    if (pull <= 0) return { ...from };
    if (pull >= 1) return { ...center };
    const dx = from.x - center.x;
    const dy = from.y - center.y;
    const r = Math.hypot(dx, dy) * (1 - pull);
    const a = Math.atan2(dy, dx) + 0.5 * pull * pull;
    return { x: center.x + Math.cos(a) * r, y: center.y + Math.sin(a) * r };
}

/** 흩뿌려지는 낱말의 자리: 낱말마다 정해 준 방향(angle)·거리(dist)로, 처음엔 빠르고 끝에서 느려진다 */
export function scatterPoint(center: Point, angle: number, dist: number, burst: number): Point {
    const k = dist * (1 - (1 - clamp01(burst)) ** 3);
    return { x: center.x + Math.cos(angle) * k, y: center.y + Math.sin(angle) * k };
}

/** 화음 순간 화면 흔들림 세기 0..1 (곧 잦아든다) */
export function openingShake(ms: number): number {
    if (ms < OPENING_CHORD_MS) return 0;
    return Math.exp(-(ms - OPENING_CHORD_MS) / 90);
}

/** 후두둑: 로딩 화면이 걷히는 길이 */
export const FALL_END_MS = 900;

export interface FallFrame {
    /** 로딩 화면 바탕이 덮는 정도 1..0 */
    cover: number;
    /** 가운데 묶음이 가라앉은 정도 0..1 */
    sink: number;
    /** 떨어지는 글자의 불투명도 */
    glyphs: number;
    done: boolean;
}

export function fallFrame(ms: number): FallFrame {
    const smooth = (a: number, b: number) => {
        const x = clamp01((ms - a) / (b - a));
        return x * x * (3 - 2 * x);
    };
    return {
        cover: 1 - smooth(100, 550),
        sink: smooth(50, 450),
        glyphs: 1 - smooth(550, FALL_END_MS),
        done: ms >= FALL_END_MS,
    };
}

/** 떨어지는 글자의 자리: 제자리에서 처음 빠르기 v0(px/s)로 시작해 화면 높이에 맞춘 중력으로 떨어진다 */
export function fallPoint(from: Point, ms: number, v0: number, height: number): Point {
    const s = Math.max(0, ms) / 1000;
    return { x: from.x, y: from.y + v0 * s + 0.5 * 2.6 * height * s * s };
}

/**
 * 오프닝을 재생해도 될 때: 기다릴 것(글꼴·그림 디코딩 등)이 모두 끝나고(실패해도 끝난 것으로 친다) 화면이
 * 두 번 그려진 뒤. 하나라도 끝나지 않으면 timeoutMs 뒤에 그냥 재생한다.
 */
export function whenOpeningReady(
    waits: Promise<unknown>[],
    timeoutMs: number,
    nextFrame: (cb: () => void) => void,
): Promise<void> {
    const settled = Promise.allSettled(waits);
    const timeout = new Promise<void>((resolve) => setTimeout(resolve, timeoutMs));
    return Promise.race([settled, timeout]).then(
        () => new Promise<void>((resolve) => nextFrame(() => nextFrame(() => resolve()))),
    );
}
