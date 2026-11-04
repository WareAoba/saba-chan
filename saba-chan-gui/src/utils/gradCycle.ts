/**
 * 그라데이션 버튼 호버 순환
 * 마우스를 올리면 그라데이션이 흐르고, 내리면 원래 그림이 다시 오는 자리에 부드럽게 선다.
 * 한 번 움직이면 최소 한 바퀴는 돈다. 그림은 CSS 의 --gs(0~200%) 하나로 그린다 (styles/grad-cycle.css)
 */

/** 순환하는 버튼. styles/grad-cycle.css 의 선택자와 같아야 한다 (강조색 그라데이션 설정이 켜졌을 때만) */
export const GRAD_CYCLE_SELECTOR =
    ':root[data-brand-gradient="on"] :is(.btn-primary, .btn.btn-confirm, .console-send, .background-restart-btn, .btn-accent)';

export const LAP_SEC = 2.4; // 최고 속도에서 한 바퀴 시간
const RAMP_SEC = 0.6; // 0 → 최고 속도 (S자 곡선)
const BRAKE_SEC = 0.9; // 최고 속도 → 0 (끝에서 가속도도 0)
const VMAX = 1 / LAP_SEC;
const smooth = (u: number) => u * u * (3 - 2 * u);

/** 속도 v 가 되는 가속 진행도 (smooth 의 역함수) */
function throttleOf(v: number) {
    let lo = 0;
    let hi = 1;
    for (let i = 0; i < 20; i++) {
        const m = (lo + hi) / 2;
        if (smooth(m) * VMAX < v) lo = m;
        else hi = m;
    }
    return hi;
}

/**
 * 순수한 움직임 계산. pos 는 시작점에서 돈 거리(바퀴), vel 은 바퀴/초.
 * step(dt, hover) 는 아직 움직이면 true 를 돌려주고, 멈추면 pos 를 0 으로 되돌린다.
 */
export function createCycleMotion() {
    let pos = 0;
    let vel = 0;
    let u = 0;
    let target: number | null = null;
    let brake: { p0: number; d: number; v0: number; t: number; T: number } | null = null;
    const speedUp = (dt: number) => {
        u = Math.min(1, u + dt / RAMP_SEC);
        vel = VMAX * smooth(u);
    };
    return {
        get pos() {
            return pos;
        },
        get vel() {
            return vel;
        },
        get moving() {
            return pos > 0 || vel > 0;
        },
        step(dt: number, hover: boolean) {
            if (hover) {
                if (brake) {
                    u = throttleOf(vel);
                    brake = null;
                }
                target = null;
                speedUp(dt);
                pos += vel * dt;
                return true;
            }
            if (pos === 0 && vel === 0) return false;
            if (!brake) {
                // 감속 거리(v·BRAKE/3) 너머 가장 가까운 정수 바퀴. 최소 한 바퀴
                if (target === null) target = Math.max(1, Math.ceil(pos + (vel * BRAKE_SEC) / 3 - 1e-9));
                const rest = target - pos;
                if (vel > 0 && rest <= (vel * BRAKE_SEC) / 3) {
                    brake = { p0: pos, d: rest, v0: vel, t: 0, T: (3 * rest) / vel };
                } else {
                    speedUp(dt);
                    pos = Math.min(pos + vel * dt, target);
                    return true;
                }
            }
            // 끝에서 속도·가속도가 함께 0 이 되는 곡선: p = p0 + d·(1 - (1-s)³)
            brake.t += dt;
            const s = Math.min(1, brake.t / brake.T);
            const k = 1 - s;
            pos = brake.p0 + brake.d * (1 - k * k * k);
            vel = brake.v0 * k * k;
            if (s >= 1) {
                pos = 0;
                vel = 0;
                u = 0;
                target = null;
                brake = null;
                return false;
            }
            return true;
        },
    };
}

/**
 * selector 에 맞는 요소에 마우스를 올리면 순환을 돌린다 (문서 하나에 위임해서 한 번만 건다).
 * 터치는 호버가 없으니 움직이지 않는다. 동작 줄이기를 켠 기기와 비활성 버튼도 움직이지 않는다
 */
interface CycleState {
    m: ReturnType<typeof createCycleMotion>;
    hover: boolean;
    raf: number;
    last: number;
}

export function installGradCycle(doc: Document = document, selector = GRAD_CYCLE_SELECTOR) {
    const win = doc.defaultView;
    if (!win?.requestAnimationFrame) return;
    const reduce = win.matchMedia?.('(prefers-reduced-motion: reduce)');
    const states = new WeakMap<HTMLElement, CycleState>();

    const tick = (el: HTMLElement) => (now: number) => {
        const st = states.get(el)!;
        const dt = Math.max(0, Math.min(0.05, (now - st.last) / 1000));
        st.last = now;
        const going = st.m.step(dt, st.hover);
        el.style.setProperty('--gs', `${((st.m.pos % 1) * 200).toFixed(3)}%`);
        if (going) {
            st.raf = win.requestAnimationFrame(tick(el));
        } else {
            st.raf = 0;
            el.style.removeProperty('--gs');
        }
    };
    const find = (t: EventTarget | null) => (t instanceof win.Element ? (t.closest(selector) as HTMLElement | null) : null);

    doc.addEventListener('pointerover', (e) => {
        const el = find(e.target);
        if (!el || e.pointerType === 'touch' || reduce?.matches) return;
        if (el.matches(':disabled, [aria-disabled="true"]') || el.contains(e.relatedTarget as Node | null)) return;
        let st = states.get(el);
        if (!st) {
            st = { m: createCycleMotion(), hover: false, raf: 0, last: 0 };
            states.set(el, st);
        }
        st.hover = true;
        if (!st.raf) {
            st.last = win.performance.now();
            st.raf = win.requestAnimationFrame(tick(el));
        }
    });
    doc.addEventListener('pointerout', (e) => {
        const el = find(e.target);
        if (!el || el.contains(e.relatedTarget as Node | null)) return;
        const st = states.get(el);
        if (st) st.hover = false;
    });
}
