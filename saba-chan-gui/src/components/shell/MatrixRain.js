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
const fallbackGlyph = () => (Math.random() < 0.18 ? pick(WORDS) : pick(HEX) + pick(HEX));

// 사람·비밀값이 섞일 수 있는 줄은 통째로 쓰지 않는다 (로딩 화면은 캡처되어 공유된다)
const SENSITIVE_LINE =
    /\b(audit|users?|discord|member|owner|token|keys?|password|passwd|secret|auth\w*|credentials?|login|e-?mail|ip)\b|@/i;
// 평범한 영어 낱말만: 소문자, 첫 글자만 대문자, 짧은 전부 대문자 (숫자·경로·주소·섞인 대소문자는 빠진다)
const PLAIN_WORD = /^(?:[a-z]{2,12}|[A-Z][a-z]{1,11}|[A-Z]{2,6})$/;
const MAX_TOKENS = 200;

/** 데몬 로그 줄에서 매트릭스에 흘릴 낱말을 뽑는다 (겹치지 않게, 최근 것 위주로) */
export function logTokens(lines) {
    const seen = new Set();
    for (const line of lines || []) {
        const message = String(line?.message || '');
        if (SENSITIVE_LINE.test(message)) continue;
        for (const raw of message.split(/\s+/)) {
            const word = raw.replace(/^[[("'`]+|[\])"'`.,:;!?]+$/g, '');
            if (PLAIN_WORD.test(word)) seen.add(word);
        }
    }
    return [...seen].slice(-MAX_TOKENS);
}

function readColor(el, name, fallback) {
    const v = getComputedStyle(el).getPropertyValue(name).trim();
    return v || fallback;
}

/**
 * 콘솔창 매트릭스: 로그 조각이 세로로 흘러내린다.
 * tokens(데몬 로그에서 뽑은 낱말)가 있으면 그것을, 없으면(데몬이 아직 안 떴으면) 기본 낱말을 흘린다.
 * 줄기는 정해진 칸이 아니라 아무 자리에 흩뿌리고, 다시 내려올 때마다 자리를 새로 고른다.
 * 움직임을 끄거나 줄이면 한 장면만 그려 두고 멈춘다.
 */
export function MatrixRain({ className, tokens }) {
    const ref = useRef(null);
    // 낱말이 바뀌어도 애니메이션은 다시 시작하지 않는다
    const tokensRef = useRef(tokens);
    tokensRef.current = tokens;

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
        const moving = motionAllowed();
        const ink = readColor(canvas, '--matrix-ink', '#7d8ee8');
        const head = readColor(canvas, '--matrix-head', '#e6e9ff');
        const fade = readColor(canvas, '--matrix-fade', 'rgba(13, 14, 16, 0.16)');
        const glyph = () => {
            const words = tokensRef.current;
            return words?.length ? pick(words) : fallbackGlyph();
        };

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
            cols = Array.from({ length: Math.ceil(w / gap) }, () => fresh(true));
            // 크기를 바꾸면 캔버스가 지워진다: 멈춘 화면이면 한 장면을 다시 채운다
            if (!moving) prefill();
        };

        // 한 장면만: 몇 번 미리 흘려서 화면을 채운다
        const prefill = () => {
            for (let i = 0; i < 60; i++) step();
        };

        // 열마다 한 줄씩 내려가되 빠르기(몇 걸음마다 한 줄)가 다르다
        const fresh = (anywhere) => ({
            x: Math.random() * w,
            // 처음에는 화면 높이 전체에 흩어 둔다 (모두 위에서 시작하면 짧은 로딩 동안 위쪽에만 몰린다)
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
                // 새 머리 글자를 밝게 쓰고, 지나간 줄은 반투명 바탕에 덮이며 차츰 흐려진다
                c.row += 1;
                const y = c.row * LINE;
                if (y >= 0) {
                    const text = glyph();
                    ctx.fillStyle = Math.random() < 0.85 ? ink : head;
                    ctx.fillText(text, c.x, y);
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

        resize();
        if (moving) raf = requestAnimationFrame(loop);
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
