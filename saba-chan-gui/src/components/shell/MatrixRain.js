import { useEffect, useRef } from 'react';
import { motionAllowed } from '../../hooks/useSwapAnimation';

// 서버 콘솔에서 흘러갈 법한 조각들: 짧은 로그 낱말과 16진수
const WORDS = [
    'INFO',
    'WARN',
    'OK',
    'PID',
    'TCP',
    'UDP',
    'RCON',
    'JVM',
    'tick',
    'save',
    'world',
    'spawn',
    'join',
    'port',
    'ping',
    'sync',
    'boot',
    'core',
    'saba',
    'heap',
    'gc',
    'tps',
    '25565',
    '8211',
];
const HEX = '0123456789abcdef';
const FONT_PX = 10;
const STEP_MS = 70;
const LINE = Math.round(FONT_PX * 1.5);

const pick = (s) => s[Math.floor(Math.random() * s.length)];
const glyph = () => (Math.random() < 0.18 ? pick(WORDS) : pick(HEX) + pick(HEX));

function readColor(el, name, fallback) {
    const v = getComputedStyle(el).getPropertyValue(name).trim();
    return v || fallback;
}

/**
 * 콘솔창 매트릭스: 로그 조각이 세로로 흘러내린다.
 * 움직임을 끄거나 줄이면 한 장면만 그려 두고 멈춘다.
 */
export function MatrixRain({ className }) {
    const ref = useRef(null);

    useEffect(() => {
        const canvas = ref.current;
        const ctx = canvas?.getContext('2d');
        if (!ctx) return undefined;

        let cols = [];
        let rows = 0;
        let w = 0;
        let h = 0;
        let raf = 0;
        let last = 0;
        const ink = readColor(canvas, '--matrix-ink', '#7d8ee8');
        const head = readColor(canvas, '--matrix-head', '#e6e9ff');
        const fade = readColor(canvas, '--matrix-fade', 'rgba(13, 14, 16, 0.16)');

        const resize = () => {
            const dpr = Math.min(window.devicePixelRatio || 1, 2);
            w = canvas.clientWidth;
            h = canvas.clientHeight;
            canvas.width = Math.round(w * dpr);
            canvas.height = Math.round(h * dpr);
            ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
            ctx.font = `${FONT_PX}px ${readColor(canvas, '--font-mono', 'monospace')}`;
            ctx.textBaseline = 'top';
            const gap = FONT_PX * 4.2;
            rows = Math.ceil(h / LINE);
            cols = Array.from({ length: Math.ceil(w / gap) }, (_, i) => fresh(i * gap, true));
        };

        // 열마다 한 줄씩 내려가되 빠르기(몇 걸음마다 한 줄)가 다르다
        const fresh = (x, anywhere) => ({
            x,
            row: anywhere ? -Math.floor(Math.random() * rows) : -Math.floor(Math.random() * 12),
            every: 1 + Math.floor(Math.random() * 3),
            tick: 0,
        });

        const step = () => {
            ctx.fillStyle = fade;
            ctx.fillRect(0, 0, w, h);
            for (const c of cols) {
                c.tick += 1;
                if (c.tick % c.every) continue;
                // 새 머리 글자를 밝게 쓰고, 지나간 줄은 반투명 바탕에 덮이며 차츰 흐려진다
                c.row += 1;
                const y = c.row * LINE;
                if (y >= 0) {
                    const text = glyph();
                    ctx.fillStyle = Math.random() < 0.85 ? ink : head;
                    ctx.fillText(text, c.x, y);
                }
                if (c.row > rows + 2 && Math.random() < 0.1) Object.assign(c, fresh(c.x, false));
            }
        };

        const loop = (now) => {
            raf = requestAnimationFrame(loop);
            if (document.hidden || now - last < STEP_MS) return;
            last = now;
            step();
        };

        resize();
        const moving = motionAllowed();
        if (moving) {
            raf = requestAnimationFrame(loop);
        } else {
            // 한 장면만: 몇 번 미리 흘려서 화면을 채운다
            for (let i = 0; i < 60; i++) step();
        }
        window.addEventListener('resize', resize);
        return () => {
            cancelAnimationFrame(raf);
            window.removeEventListener('resize', resize);
        };
    }, []);

    return (
        <div className={className} aria-hidden="true">
            <canvas ref={ref} />
        </div>
    );
}
