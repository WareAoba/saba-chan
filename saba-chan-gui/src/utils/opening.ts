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
 * 오프닝 애니메이션: 로딩 화면에 실제로 떠 있던 콘솔 글자를 가운데로 쭉 끌어모으다가, 기동음의 마지막 음(화음)에
 * 섬광과 함께 메인 화면을 열고 글자를 온 방향으로 다시 흩뿌린다. 메인 화면은 재생 전에 로딩 화면 아래에 다 그려 둔다.
 */

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

/** 터져 나가는 글자의 자리: 끌려 온 방향 그대로, 처음 자리보다 멀리 */
export function burstPoint(from: Point, center: Point, burst: number): Point {
    const k = 1.4 * (1 - (1 - clamp01(burst)) ** 3);
    return { x: center.x + (from.x - center.x) * k, y: center.y + (from.y - center.y) * k };
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
