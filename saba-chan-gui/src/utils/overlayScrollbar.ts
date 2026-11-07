// 앱 전체 오버레이 스크롤바.
// 기본 스크롤바는 CSS(base.css)에서 숨기고, 사람이 실제로 스크롤한 영역 위에만
// 얇은 손잡이 하나를 띄운다 (포인터를 올리기만 해서는 뜨지 않는다, 사용자 11-07).
// 손잡이는 내용 위에 겹쳐 그려서 레이아웃을 밀지 않는다.

const HIDE_MS = 900;
const MIN_THUMB = 24;
const EDGE = 2;
/** 휠·터치·키·끌기 뒤 이 시간 안의 스크롤만 사람이 한 것으로 본다 (콘솔 자동 내림 같은 코드 스크롤은 손잡이를 띄우지 않는다) */
const INTENT_MS = 800;

const scrollableY = (el: HTMLElement) => {
    if (el.scrollHeight <= el.clientHeight + 1) return false;
    const o = getComputedStyle(el).overflowY;
    return o === 'auto' || o === 'scroll';
};
const scrollableX = (el: HTMLElement) => {
    if (el.scrollWidth <= el.clientWidth + 1) return false;
    const o = getComputedStyle(el).overflowX;
    return o === 'auto' || o === 'scroll';
};

/**
 * 스크롤 영역 중 화면에 실제로 보이는 사각형: 창과, 내용을 잘라 내는 바깥 상자들(overflow가 visible이 아닌 것)의 안쪽과 겹치는 부분.
 * 손잡이를 이 안으로만 그려 다른 요소 위로 뚫고 나오지 않게 한다 (사용자 11-07)
 */
function visibleRect(el: HTMLElement) {
    const r = el.getBoundingClientRect();
    let left = Math.max(0, r.left);
    let top = Math.max(0, r.top);
    let right = Math.min(window.innerWidth, r.right);
    let bottom = Math.min(window.innerHeight, r.bottom);
    for (let p = el.parentElement; p && p !== document.body && p !== document.documentElement; p = p.parentElement) {
        const cs = getComputedStyle(p);
        if (cs.position === 'fixed') break;
        if (cs.overflowX === 'visible' && cs.overflowY === 'visible') continue;
        const pr = p.getBoundingClientRect();
        const pl = pr.left + p.clientLeft;
        const pt = pr.top + p.clientTop;
        if (cs.overflowX !== 'visible') {
            left = Math.max(left, pl);
            right = Math.min(right, pl + p.clientWidth);
        }
        if (cs.overflowY !== 'visible') {
            top = Math.max(top, pt);
            bottom = Math.min(bottom, pt + p.clientHeight);
        }
    }
    return { left, top, right, bottom };
}

/** 손잡이 상자(x, y, w, h) 중 보이는 사각형 밖을 잘라 낸다. 다 잘리면 false */
function clipTo(thumb: HTMLElement, x: number, y: number, w: number, h: number, v: ReturnType<typeof visibleRect>) {
    const l = Math.max(0, v.left - x);
    const t = Math.max(0, v.top - y);
    const r = Math.max(0, x + w - v.right);
    const b = Math.max(0, y + h - v.bottom);
    if (l + r >= w || t + b >= h) return false;
    thumb.style.clipPath = l || t || r || b ? `inset(${t}px ${r}px ${b}px ${l}px round 999px)` : '';
    return true;
}

/** 이 요소가 화면에서 실제로 보이는가 (모달 등에 덮이지 않았는가) */
function visibleAt(el: HTMLElement) {
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return false;
    const hit = document.elementFromPoint(r.left + r.width / 2, r.top + Math.min(r.height / 2, 20));
    return !!hit && (el === hit || el.contains(hit));
}

export function installOverlayScrollbars() {
    if (typeof document === 'undefined' || document.documentElement.dataset.overlayScroll) return;
    document.documentElement.dataset.overlayScroll = 'on';

    const make = (axis: 'x' | 'y') => {
        const t = document.createElement('div');
        t.className = `ovs-thumb ovs-${axis}`;
        t.setAttribute('aria-hidden', 'true');
        document.body.appendChild(t);
        return t;
    };
    const ty = make('y');
    const tx = make('x');

    let target: HTMLElement | null = null;
    let hideTimer: ReturnType<typeof setTimeout> | undefined;
    let drag: { axis: 'x' | 'y'; start: number; scroll: number } | null = null;

    const place = () => {
        if (!target || !target.isConnected) {
            ty.classList.remove('show');
            tx.classList.remove('show');
            return;
        }
        const r = target.getBoundingClientRect();
        const vis = visibleRect(target);
        // 손잡이 폭 (CSS의 6px와 margin -6px: 오른쪽·아래 끝에서 안쪽으로 들어온 자리)
        const T = 6;
        // 세로
        if (scrollableY(target)) {
            const ch = target.clientHeight;
            const h = Math.max(MIN_THUMB, (ch * ch) / target.scrollHeight);
            const max = target.scrollHeight - ch;
            const top = r.top + target.clientTop + (max > 0 ? (target.scrollTop / max) * (ch - h) : 0);
            ty.style.transform = `translate(${r.left + target.clientLeft + target.clientWidth - EDGE}px, ${top}px)`;
            ty.style.height = `${h}px`;
            const x = r.left + target.clientLeft + target.clientWidth - EDGE - T;
            if (clipTo(ty, x, top, T, h, vis)) ty.classList.add('show');
            else ty.classList.remove('show');
        } else ty.classList.remove('show');
        // 가로
        if (scrollableX(target)) {
            const cw = target.clientWidth;
            const w = Math.max(MIN_THUMB, (cw * cw) / target.scrollWidth);
            const max = target.scrollWidth - cw;
            const left = r.left + target.clientLeft + (max > 0 ? (target.scrollLeft / max) * (cw - w) : 0);
            tx.style.transform = `translate(${left}px, ${r.top + target.clientTop + target.clientHeight - EDGE}px)`;
            tx.style.width = `${w}px`;
            const y = r.top + target.clientTop + target.clientHeight - EDGE - T;
            if (clipTo(tx, left, y, w, T, vis)) tx.classList.add('show');
            else tx.classList.remove('show');
        } else tx.classList.remove('show');
    };

    const scheduleHide = (): void => {
        clearTimeout(hideTimer);
        hideTimer = setTimeout(() => {
            if (drag || ty.matches(':hover') || tx.matches(':hover')) return scheduleHide();
            ty.classList.remove('show');
            tx.classList.remove('show');
        }, HIDE_MS);
    };

    const showFor = (el: HTMLElement) => {
        target = el;
        place();
        scheduleHide();
    };

    // 사람이 스크롤하려고 한 때 (휠·터치·스크롤 키·글자 끌어 고르기)
    let lastIntent = -Infinity;
    const intent = () => {
        lastIntent = performance.now();
    };
    for (const type of ['wheel', 'touchmove', 'pointerdown'] as const) {
        document.addEventListener(type, intent, { passive: true, capture: true });
    }
    const SCROLL_KEYS = new Set(['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'PageUp', 'PageDown', 'Home', 'End', ' ']);
    document.addEventListener('keydown', (e) => SCROLL_KEYS.has(e.key) && intent(), { capture: true });

    document.addEventListener(
        'scroll',
        (e) => {
            const el = e.target === document ? null : e.target;
            if (!(el instanceof HTMLElement)) return;
            // 보이는 손잡이는 어떤 스크롤에도 자리를 따라간다. 새로 띄우는 것은 사람이 스크롤했을 때만
            if (el === target && (ty.classList.contains('show') || tx.classList.contains('show'))) {
                if (drag || performance.now() - lastIntent < INTENT_MS) showFor(el);
                else place();
                return;
            }
            if (performance.now() - lastIntent < INTENT_MS && visibleAt(el)) showFor(el);
        },
        true,
    );
    window.addEventListener('resize', place);

    // 손잡이를 끌어서 스크롤
    const startDrag = (axis: 'x' | 'y') => (e: PointerEvent) => {
        if (!target) return;
        e.preventDefault();
        const thumb = axis === 'y' ? ty : tx;
        thumb.setPointerCapture?.(e.pointerId);
        thumb.classList.add('drag');
        drag = {
            axis,
            start: axis === 'y' ? e.clientY : e.clientX,
            scroll: axis === 'y' ? target.scrollTop : target.scrollLeft,
        };
        const move = (ev: PointerEvent) => {
            if (!drag || !target) return;
            if (axis === 'y') {
                const ch = target.clientHeight;
                const h = Math.max(MIN_THUMB, (ch * ch) / target.scrollHeight);
                const ratio = (target.scrollHeight - ch) / Math.max(1, ch - h);
                target.scrollTop = drag.scroll + (ev.clientY - drag.start) * ratio;
            } else {
                const cw = target.clientWidth;
                const w = Math.max(MIN_THUMB, (cw * cw) / target.scrollWidth);
                const ratio = (target.scrollWidth - cw) / Math.max(1, cw - w);
                target.scrollLeft = drag.scroll + (ev.clientX - drag.start) * ratio;
            }
        };
        const up = () => {
            drag = null;
            thumb.classList.remove('drag');
            thumb.removeEventListener('pointermove', move);
            thumb.removeEventListener('pointerup', up);
            thumb.removeEventListener('pointercancel', up);
            scheduleHide();
        };
        thumb.addEventListener('pointermove', move);
        thumb.addEventListener('pointerup', up);
        thumb.addEventListener('pointercancel', up);
    };
    ty.addEventListener('pointerdown', startDrag('y'));
    tx.addEventListener('pointerdown', startDrag('x'));
}
