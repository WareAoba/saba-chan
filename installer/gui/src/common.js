// ═══════════════════════════════════════════════════════
// 설치기·업데이터 창 공용: 테마, 글자 로고, 링, 매트릭스, 그림 펼침 흐름
// 번들러 없이 실리므로 window.SabaUI 하나로 내보낸다 (installer.js·updater.js보다 먼저 싣는다)
// ═══════════════════════════════════════════════════════

window.SabaUI = (() => {
    const REDUCE = !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

    // 그림 펼침 0.8초 · 펼친 채 머묾 1.2초 · 접힘 0.8초 (CSS --t-*는 여기서 넣는다)
    const TIMING = { open: 800, hold: 1200, fold: 800 };
    // 완료 화면 전환(매트릭스 끄기 0.6초 · 링 색 0.8초)이 끝나는 때
    const SETTLE_MS = 900;

    /** 테마: 'light' | 'dark' | 'auto'. 토큰이 :root에서 다른 토큰을 참조하므로 html에 건다 */
    function setTheme(theme) {
        document.documentElement.setAttribute('data-theme', theme);
    }

    function systemTheme() {
        return window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
    }

    /** 지금 보이는 테마 ('auto'면 시스템 설정) */
    function effectiveTheme() {
        const t = document.documentElement.getAttribute('data-theme');
        return t === 'light' || t === 'dark' ? t : systemTheme();
    }

    /**
     * 창 바탕 반투명 (앱 설정 → 외관을 따른다, src-tauri/src/backdrop.rs). 셸이 OS 효과를 깔았으면 바탕을 비우고
     * 테마 바탕을 앱과 같은 진하기·밝기로 덮는다 (common.css data-backdrop)
     */
    async function applyBackdrop() {
        const invoke = window.__TAURI__?.core?.invoke;
        if (!invoke) return;
        try {
            const b = await invoke('get_window_backdrop');
            if (!b?.on) return;
            const root = document.documentElement;
            root.style.setProperty('--backdrop-opacity', `${b.opacity}%`);
            root.style.setProperty('--backdrop-mix', b.brightness >= 0 ? 'white' : 'black');
            root.style.setProperty('--backdrop-mix-amount', `${Math.abs(b.brightness)}%`);
            root.dataset.backdrop = 'on';
        } catch (e) {
            console.warn('window backdrop', e);
        }
    }
    applyBackdrop();

    /** 언어에 맞는 글자 로고 (GUI와 같은 규칙: 한국어·일본어 전용, 나머지 영어) */
    function setWordmark(lang) {
        const file = lang.startsWith('ko') ? 'logo-kr.png' : lang.startsWith('ja') ? 'logo-jp.png' : 'logo-en.png';
        // CSS 변수 속 url()은 쓰는 쪽 기준으로 풀리므로 문서 기준 절대 주소로 넣는다
        document.documentElement.style.setProperty('--wordmark', `url("${new URL(file, document.baseURI).href}")`);
    }

    // ── 링 ─────────────────────────────────────────────
    // 한 바퀴 1초짜리 회전을 두고 playbackRate로 빠르기를 바꾼다 (각도가 끊기지 않는다)
    const PERIOD = { idle: 6000, spin: 1000, ok: 3000, bad: 2000 };

    function ring(el) {
        const parts = REDUCE || !el.animate ? [] : [...el.querySelectorAll('.g1, .g2')];
        const anims = parts.map((g) =>
            g.animate([{ transform: 'rotate(0deg)' }, { transform: 'rotate(360deg)' }], { duration: 1000, iterations: Infinity }),
        );
        let state = Object.keys(PERIOD).find((k) => el.classList.contains(k)) || 'idle';
        let rate = 1000 / PERIOD[state];
        let raf = 0;
        anims.forEach((a) => { a.playbackRate = rate; });

        function set(next) {
            if (!(next in PERIOD)) return;
            state = next;
            for (const k of Object.keys(PERIOD)) el.classList.toggle(k, k === next);
            if (!anims.length) return;
            // 0.6초에 걸쳐 새 빠르기로
            const from = rate;
            const to = 1000 / PERIOD[next];
            const t0 = performance.now();
            cancelAnimationFrame(raf);
            const step = (now) => {
                const p = Math.min(1, (now - t0) / 600);
                rate = from + (to - from) * (1 - (1 - p) ** 3);
                anims.forEach((a) => { a.playbackRate = rate; });
                if (p < 1) raf = requestAnimationFrame(step);
            };
            raf = requestAnimationFrame(step);
        }
        return { set, get state() { return state; } };
    }

    // ── 매트릭스 ───────────────────────────────────────
    // GUI 로딩 화면의 MatrixRain(saba-chan-gui/src/components/shell/MatrixRain.tsx)과 같은 방식:
    // 열마다 낱말이 흘러내리고, 지나간 줄은 반투명 바탕에 덮이며 흐려진다
    const WORDS = ['saba', 'core', 'gui', 'cli', 'py', 'toml', 'json', 'sha256', 'OK', 'copy', 'unzip', 'link', 'reg', 'module', 'ext', 'locale', 'runtime', 'lnk'];
    const HEX = '0123456789abcdef';
    const FONT_PX = 10;
    const LINE = Math.round(FONT_PX * 1.5);
    const STEP_MS = 70;
    const pick = (s) => s[Math.floor(Math.random() * s.length)];
    const glyph = () => (Math.random() < 0.18 ? pick(WORDS) : pick(HEX) + pick(HEX));

    /** CSS 색 값(변수·color-mix 포함)을 캔버스가 받는 rgb()로 */
    function resolveColor(host, name, fallback) {
        const probe = document.createElement('i');
        probe.style.color = `var(${name}, ${fallback})`;
        host.appendChild(probe);
        const c = getComputedStyle(probe).color;
        probe.remove();
        return c || fallback;
    }

    function matrix(box) {
        const canvas = box.querySelector('canvas');
        let ctx = null;
        let cols = [];
        let rows = 0;
        let w = 0;
        let h = 0;
        let raf = 0;
        let last = 0;
        let ink = '';
        let head = '';
        let fade = '';

        const fresh = (anywhere) => ({
            x: Math.random() * w,
            row: anywhere ? Math.floor(Math.random() * rows) : -Math.floor(Math.random() * 12),
            every: 1 + Math.floor(Math.random() * 3),
            tick: 0,
        });

        const step = () => {
            // 반투명 창이면 바탕색을 덧칠하지 않고 지난 글자를 지운다 (캔버스가 불투명해지지 않게)
            const clear = document.documentElement.dataset.backdrop === 'on';
            if (clear) ctx.globalCompositeOperation = 'destination-out';
            ctx.fillStyle = clear ? 'rgba(0, 0, 0, 0.14)' : fade;
            ctx.fillRect(0, 0, w, h);
            ctx.globalCompositeOperation = 'source-over';
            for (const c of cols) {
                c.tick += 1;
                if (c.tick % c.every) continue;
                c.row += 1;
                const y = c.row * LINE;
                if (y >= 0) {
                    ctx.fillStyle = Math.random() < 0.85 ? ink : head;
                    ctx.fillText(glyph(), c.x, y);
                }
                if (c.row > rows + 2 && Math.random() < 0.1) Object.assign(c, fresh(false));
            }
        };

        const loop = (now) => {
            raf = requestAnimationFrame(loop);
            if (document.hidden || now - last < STEP_MS) return;
            last = now;
            step();
        };

        function start() {
            if (raf) return;
            ctx = ctx || canvas?.getContext?.('2d') || null;
            if (!ctx) return;
            // 테마를 바꿨을 수 있으니 시작할 때마다 색을 다시 읽는다
            ink = resolveColor(box, '--matrix-ink', '#7d8ee8');
            head = resolveColor(box, '--matrix-head', '#e6e9ff');
            fade = resolveColor(box, '--matrix-fade', 'rgba(13, 14, 16, 0.16)');
            const dpr = Math.min(window.devicePixelRatio || 1, 2);
            w = canvas.clientWidth;
            h = canvas.clientHeight;
            canvas.width = Math.round(w * dpr);
            canvas.height = Math.round(h * dpr);
            ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
            ctx.font = `${FONT_PX}px ${getComputedStyle(box).getPropertyValue('--font-mono') || 'monospace'}`;
            ctx.textBaseline = 'top';
            rows = Math.ceil(h / LINE);
            cols = Array.from({ length: Math.ceil(w / (FONT_PX * 4.2)) }, () => fresh(true));
            // 움직임을 줄였으면 한 장면만 채워 두고 멈춘다
            if (REDUCE) {
                for (let i = 0; i < 60; i++) step();
                return;
            }
            raf = requestAnimationFrame(loop);
        }

        function stop() {
            cancelAnimationFrame(raf);
            raf = 0;
        }
        return { start, stop };
    }

    // ── 그림 펼침 흐름 ─────────────────────────────────
    // expand(펼쳐 머묾) 동안 끝난 결과는 접히기 시작할 때 보인다: 그림이 덮은 채로 화면이 바뀌지 않게
    function flow(el, { onFold } = {}) {
        el.style.setProperty('--t-open', `${TIMING.open}ms`);
        el.style.setProperty('--t-hold', `${TIMING.hold}ms`);
        el.style.setProperty('--t-fold', `${TIMING.fold}ms`);
        let timer = 0;
        let waiting = [];
        let foldAt = -Infinity;
        let laterTimer = 0;

        function go(step) {
            el.dataset.step = step;
        }

        function fold() {
            clearTimeout(timer);
            timer = 0;
            foldAt = performance.now();
            go('install');
            onFold?.();
            const run = waiting;
            waiting = [];
            run.forEach((fn) => fn());
        }

        /** 그림을 펼쳐 머문 뒤 접으며 매트릭스로. extraMs는 펼침 앞에 더 기다릴 시간 */
        function reveal(extraMs = 0) {
            go('expand');
            if (REDUCE) { fold(); return; }
            timer = setTimeout(fold, extraMs + TIMING.open + TIMING.hold);
        }

        /** 펼친 그림이 접히기 시작한 뒤에 (이미 접혔으면 바로) */
        function afterReveal(fn) {
            if (timer) waiting.push(fn);
            else fn();
        }

        /**
         * 완료 화면으로 다 넘어간 뒤에 (매트릭스가 꺼지고 링 색이 바뀐 뒤, 그림이 아직 접히는 중이면 다 접힌 뒤).
         * 경고처럼 링 영역을 밀어내며 나오는 것을 이때 띄운다
         */
        function afterSettled(fn) {
            clearTimeout(laterTimer);
            if (REDUCE) { fn(); return; }
            const folding = TIMING.fold - (performance.now() - foldAt);
            laterTimer = setTimeout(fn, Math.max(SETTLE_MS, folding + 300));
        }

        function reset(step) {
            clearTimeout(laterTimer);
            clearTimeout(timer);
            timer = 0;
            waiting = [];
            go(step);
        }

        return { go, reveal, afterReveal, afterSettled, reset, get step() { return el.dataset.step; } };
    }

    return { REDUCE, TIMING, setTheme, systemTheme, effectiveTheme, setWordmark, ring, matrix, flow };
})();

// ── 오버레이 스크롤바 (saba-chan-gui utils/overlayScrollbar.ts와 같은 동작) ──
// 기본 스크롤바는 CSS(common.css)에서 숨기고, 포인터가 올라간 스크롤 영역 위에
// 얇은 손잡이 하나를 띄운다. 손잡이는 내용 위에 겹쳐 그려서 레이아웃을 밀지 않는다.
(() => {
    if (document.documentElement.dataset.overlayScroll) return;
    document.documentElement.dataset.overlayScroll = 'on';

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

    /** 이 요소가 화면에서 실제로 보이는가 (시트 등에 덮이지 않았는가) */
    function visibleAt(el) {
        const r = el.getBoundingClientRect();
        if (r.width === 0 || r.height === 0) return false;
        const hit = document.elementFromPoint(r.left + r.width / 2, r.top + Math.min(r.height / 2, 20));
        return !!hit && (el === hit || el.contains(hit));
    }

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
    let hideTimer;
    let drag = null;

    const place = () => {
        if (!target || !target.isConnected) {
            ty.classList.remove('show');
            tx.classList.remove('show');
            return;
        }
        const r = target.getBoundingClientRect();
        if (scrollableY(target)) {
            const ch = target.clientHeight;
            const h = Math.max(MIN_THUMB, (ch * ch) / target.scrollHeight);
            const max = target.scrollHeight - ch;
            const top = r.top + target.clientTop + (max > 0 ? (target.scrollTop / max) * (ch - h) : 0);
            ty.style.transform = `translate(${r.left + target.clientLeft + target.clientWidth - EDGE}px, ${top}px)`;
            ty.style.height = `${h}px`;
            ty.classList.add('show');
        } else ty.classList.remove('show');
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
        drag = { start: axis === 'y' ? e.clientY : e.clientX, scroll: axis === 'y' ? target.scrollTop : target.scrollLeft };
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
})();
