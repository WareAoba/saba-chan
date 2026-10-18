// 앱 전체 오버레이 스크롤바.
// 기본 스크롤바는 CSS(base.css)에서 숨기고, 포인터가 올라간 스크롤 영역 위에
// 얇은 손잡이 하나를 띄운다. 손잡이는 내용 위에 겹쳐 그려서 레이아웃을 밀지 않는다.

const HIDE_MS = 900;
const MIN_THUMB = 24;
const EDGE = 2;

const scrollableY = (el) => {
    if (el.scrollHeight <= el.clientHeight + 1) return false;
    const o = getComputedStyle(el).overflowY;
    return o === 'auto' || o === 'scroll';
};
const scrollableX = (el) => {
    if (el.scrollWidth <= el.clientWidth + 1) return false;
    const o = getComputedStyle(el).overflowX;
    return o === 'auto' || o === 'scroll';
};

/** 포인터 아래에서 가장 가까운 스크롤 영역 (세로 우선) */
function scrollerAt(node) {
    for (let el = node; el && el !== document.documentElement; el = el.parentElement) {
        if (el instanceof HTMLElement && (scrollableY(el) || scrollableX(el))) return el;
    }
    return null;
}

/** 이 요소가 화면에서 실제로 보이는가 (모달 등에 덮이지 않았는가) */
function visibleAt(el) {
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return false;
    const hit = document.elementFromPoint(r.left + r.width / 2, r.top + Math.min(r.height / 2, 20));
    return !!hit && (el === hit || el.contains(hit));
}

export function installOverlayScrollbars() {
    if (typeof document === 'undefined' || document.documentElement.dataset.overlayScroll) return;
    document.documentElement.dataset.overlayScroll = 'on';

    const make = (axis) => {
        const t = document.createElement('div');
        t.className = `ovs-thumb ovs-${axis}`;
        t.setAttribute('aria-hidden', 'true');
        document.body.appendChild(t);
        return t;
    };
    const ty = make('y');
    const tx = make('x');

    let target = null;
    let hideTimer = null;
    let drag = null;

    const place = () => {
        if (!target || !target.isConnected) {
            ty.classList.remove('show');
            tx.classList.remove('show');
            return;
        }
        const r = target.getBoundingClientRect();
        // 세로
        if (scrollableY(target)) {
            const ch = target.clientHeight;
            const h = Math.max(MIN_THUMB, (ch * ch) / target.scrollHeight);
            const max = target.scrollHeight - ch;
            const top = r.top + target.clientTop + (max > 0 ? (target.scrollTop / max) * (ch - h) : 0);
            ty.style.transform = `translate(${r.left + target.clientLeft + target.clientWidth - EDGE}px, ${top}px)`;
            ty.style.height = `${h}px`;
            ty.classList.add('show');
        } else ty.classList.remove('show');
        // 가로
        if (scrollableX(target)) {
            const cw = target.clientWidth;
            const w = Math.max(MIN_THUMB, (cw * cw) / target.scrollWidth);
            const max = target.scrollWidth - cw;
            const left = r.left + target.clientLeft + (max > 0 ? (target.scrollLeft / max) * (cw - w) : 0);
            tx.style.transform = `translate(${left}px, ${r.top + target.clientTop + target.clientHeight - EDGE}px)`;
            tx.style.width = `${w}px`;
            tx.classList.add('show');
        } else tx.classList.remove('show');
    };

    const scheduleHide = () => {
        clearTimeout(hideTimer);
        hideTimer = setTimeout(() => {
            if (drag || ty.matches(':hover') || tx.matches(':hover')) return scheduleHide();
            ty.classList.remove('show');
            tx.classList.remove('show');
        }, HIDE_MS);
    };

    const showFor = (el) => {
        target = el;
        place();
        scheduleHide();
    };

    document.addEventListener(
        'scroll',
        (e) => {
            const el = e.target === document ? null : e.target;
            if (!(el instanceof HTMLElement)) return;
            if (el === target || visibleAt(el)) showFor(el);
        },
        true,
    );

    let lastMove = 0;
    document.addEventListener(
        'pointermove',
        (e) => {
            if (drag) return;
            const now = performance.now();
            if (now - lastMove < 50) return; // 너무 자주 재지 않는다
            lastMove = now;
            if (e.target === ty || e.target === tx) return;
            const el = scrollerAt(e.target);
            if (el) showFor(el);
        },
        { passive: true },
    );
    window.addEventListener('resize', place);

    // 손잡이를 끌어서 스크롤
    const startDrag = (axis) => (e) => {
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
        const move = (ev) => {
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
