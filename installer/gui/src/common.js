// ═══════════════════════════════════════════════════════
// 설치기·업데이터 창 공용: 테마, 글자 로고, 링, 매트릭스, 그림 펼침 흐름
// 번들러 없이 실리므로 window.SabaUI 하나로 내보낸다 (installer.js·updater.js보다 먼저 싣는다)
// ═══════════════════════════════════════════════════════

window.SabaUI = (() => {
    const REDUCE = !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

    // 그림 펼침 0.8초 · 펼친 채 머묾 1.2초 · 접힘 0.8초 (CSS --t-*는 여기서 넣는다)
    const TIMING = { open: 800, hold: 1200, fold: 800 };

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
            ctx.fillStyle = fade;
            ctx.fillRect(0, 0, w, h);
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

        function go(step) {
            el.dataset.step = step;
        }

        function fold() {
            clearTimeout(timer);
            timer = 0;
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

        function reset(step) {
            clearTimeout(timer);
            timer = 0;
            waiting = [];
            go(step);
        }

        return { go, reveal, afterReveal, reset, get step() { return el.dataset.step; } };
    }

    return { REDUCE, TIMING, setTheme, systemTheme, effectiveTheme, setWordmark, ring, matrix, flow };
})();
