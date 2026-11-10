import { useEffect, useRef } from 'react';
import { motionAllowed } from '../../hooks/useSwapAnimation';
import { burstPoint, openingFrame, pullPoint } from '../../utils/opening';

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

/** 지나간 줄이 한 걸음마다 바탕에 덮여 남는 비율 (로딩 화면의 --matrix-fade 14%) */
const KEEP = 0.86;
/** 이만큼 지나면 거의 안 보이므로 기억하지 않는다 */
const MAX_AGE = 22;

/** 화면에 그려 둔 글자 하나 (오프닝이 실제로 떠 있던 글자를 끌어모으려고 기억한다) */
interface Glyph {
    text: string;
    x: number;
    y: number;
    color: string;
    age: number;
}

/** 오프닝 재생: 시작 시각(performance.now)과 글자를 끌어모을 가운데 (캔버스 기준) */
export interface MatrixPull {
    start: number;
    x: number;
    y: number;
}

/** 줄기 하나 */
interface Column {
    x: number;
    row: number;
    every: number;
    tick: number;
}

const pick = <T,>(s: ArrayLike<T>): T => s[Math.floor(Math.random() * s.length)];
const fallbackGlyph = () => (Math.random() < 0.18 ? pick(WORDS) : pick(HEX) + pick(HEX));

// 사람·비밀값이 섞일 수 있는 줄은 통째로 쓰지 않는다 (로딩 화면은 캡처되어 공유된다)
const SENSITIVE_LINE =
    /\b(audit|users?|discord|member|owner|token|keys?|password|passwd|secret|auth\w*|credentials?|login|e-?mail|ip)\b|@/i;
// 평범한 영어 낱말만: 소문자, 첫 글자만 대문자, 짧은 전부 대문자 (숫자·경로·주소·섞인 대소문자는 빠진다)
const PLAIN_WORD = /^(?:[a-z]{2,12}|[A-Z][a-z]{1,11}|[A-Z]{2,6})$/;
const MAX_TOKENS = 200;

/** 데몬 로그 줄에서 매트릭스에 흘릴 낱말을 뽑는다 (겹치지 않게, 최근 것 위주로) */
export function logTokens(lines?: { message?: unknown }[] | null) {
    const seen = new Set<string>();
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

function readColor(el: Element, name: string, fallback: string) {
    const v = getComputedStyle(el).getPropertyValue(name).trim();
    return v || fallback;
}

/**
 * 콘솔창 매트릭스: 로그 조각이 세로로 흘러내린다.
 * tokens(데몬 로그에서 뽑은 낱말)가 있으면 그것을, 없으면(데몬이 아직 안 떴으면) 기본 낱말을 흘린다.
 * 줄기는 정해진 칸이 아니라 아무 자리에 흩뿌리고, 다시 내려올 때마다 자리를 새로 고른다.
 * 움직임을 끄거나 줄이면 한 장면만 그려 두고 멈춘다.
 */
export function MatrixRain({
    className,
    tokens,
    pull,
}: {
    className?: string;
    tokens?: string[];
    /** 주면 흘리기를 멈추고 지금 떠 있는 글자를 가운데로 끌어모았다가 화음에 터뜨린다 (utils/opening.ts) */
    pull?: MatrixPull | null;
}) {
    const ref = useRef<HTMLCanvasElement>(null);
    // 낱말이 바뀌어도 애니메이션은 다시 시작하지 않는다
    const tokensRef = useRef(tokens);
    tokensRef.current = tokens;
    const pullRef = useRef(pull);
    pullRef.current = pull;

    useEffect(() => {
        const canvas = ref.current;
        const ctx = canvas?.getContext('2d');
        if (!canvas || !ctx) return undefined;

        let cols: Column[] = [];
        let glyphs: Glyph[] = [];
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
            glyphs = [];
            // 크기를 바꾸면 캔버스가 지워진다: 멈춘 화면이면 한 장면을 다시 채운다
            if (!moving) prefill();
        };

        // 한 장면만: 몇 번 미리 흘려서 화면을 채운다
        const prefill = () => {
            for (let i = 0; i < 60; i++) step();
        };

        // 열마다 한 줄씩 내려가되 빠르기(몇 걸음마다 한 줄)가 다르다
        const fresh = (anywhere: boolean): Column => ({
            x: Math.random() * w,
            // 처음에는 화면 높이 전체에 흩어 둔다 (모두 위에서 시작하면 짧은 로딩 동안 위쪽에만 몰린다)
            row: anywhere ? Math.floor(Math.random() * rows) : -Math.floor(Math.random() * 12),
            every: 1 + Math.floor(Math.random() * 3),
            tick: 0,
        });

        const step = () => {
            ctx.fillStyle = fade;
            ctx.fillRect(0, 0, w, h);
            glyphs = glyphs.filter((g) => ++g.age < MAX_AGE);
            for (const c of cols) {
                c.tick += 1;
                if (c.tick % c.every) continue;
                // 새 머리 글자를 밝게 쓰고, 지나간 줄은 반투명 바탕에 덮이며 차츰 흐려진다
                c.row += 1;
                const y = c.row * LINE;
                if (y >= 0) {
                    const text = glyph();
                    const color = Math.random() < 0.85 ? ink : head;
                    ctx.fillStyle = color;
                    ctx.fillText(text, c.x, y);
                    glyphs.push({ text, x: c.x, y, color, age: 0 });
                }
                if (c.row > rows + 2 && Math.random() < 0.1) Object.assign(c, fresh(false));
            }
        };

        // 오프닝: 떠 있던 글자를 그 밝기 그대로 가운데로 끌어모으고, 화음에 온 방향으로 흩뿌린다
        const drawPull = (p: MatrixPull, now: number) => {
            const f = openingFrame(now - p.start);
            const center = { x: p.x, y: p.y };
            ctx.clearRect(0, 0, w, h);
            if (!f.opened && f.pull > 0) {
                const g = ctx.createRadialGradient(p.x, p.y, 0, p.x, p.y, Math.max(40, Math.min(w, h) * 0.22));
                g.addColorStop(0, head);
                g.addColorStop(1, 'transparent');
                ctx.globalAlpha = 0.55 * f.pull * f.pull;
                ctx.fillStyle = g;
                ctx.fillRect(0, 0, w, h);
            }
            for (const glyph of glyphs) {
                const seen = KEEP ** glyph.age;
                const at = f.opened ? burstPoint(glyph, center, f.burst) : pullPoint(glyph, center, f.pull);
                ctx.globalAlpha = f.opened ? (1 - f.burst) ** 1.6 * Math.max(seen, 0.5) : seen + (1 - seen) * f.pull;
                ctx.fillStyle = f.opened || f.pull > 0.35 ? head : glyph.color;
                ctx.fillText(glyph.text, at.x, at.y);
            }
            ctx.globalAlpha = 1;
        };

        const loop = (now: number) => {
            raf = requestAnimationFrame(loop);
            const p = pullRef.current;
            if (p) {
                drawPull(p, now);
                return;
            }
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
