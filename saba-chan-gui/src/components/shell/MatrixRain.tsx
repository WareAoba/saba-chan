import { useEffect, useRef } from 'react';
import { motionAllowed } from '../../hooks/useSwapAnimation';
import {
    fallFrame,
    fallPoint,
    type OpeningKind,
    openingFrame,
    pullPoint,
    scatterPoint,
    wordPull,
} from '../../utils/opening';

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
/**
 * 오프닝 동안 캔버스 해상도 배수. 화면 전체 캔버스가 매 프레임 바뀌면 창 합성·표시를 기다리느라 프레임이 밀린다
 * (PC 실측: 화음 뒤 최대 프레임 24~30ms → 12ms). 글자가 빠르게 움직이는 동안이라 선명도 차이는 눈에 잘 띄지 않는다
 */
const OPENING_RESOLUTION = 0.75;
/** 터질 때 글자 조각의 처음 크기 배수 (조각 그림은 이 크기로 한 번 그려 두고 줄여 찍는다) */
const SHARD_SCALE = 1.15;

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

/** 오프닝 재생: 시작 시각(performance.now)과 글자를 끌어모을 가운데 (캔버스 기준), 정상(burst)·비정상(fall) */
export interface MatrixPull {
    start: number;
    x: number;
    y: number;
    kind: OpeningKind;
    /** burst에서 화음이 울리는 때 (opening.ts openingChordMs) */
    chord: number;
}

/** 오프닝에서 움직이는 낱말 하나: 떠 있던 자리·밝기는 그대로, 회전·흩뿌릴 방향·떨어지는 빠르기만 무작위 */
interface Word {
    text: string;
    /** 낱말 가운데 */
    x: number;
    y: number;
    width: number;
    color: string;
    seen: number;
    spin: number;
    angle: number;
    dist: number;
    v0: number;
    delay: number;
    /** 떼어 오는 차례 0..1 (가운데에서 먼 낱말부터) */
    order: number;
    /** 터질 때 낱말이 부서져 나온 글자 조각: 낱말 방향에서 비낀 각(da)·거리 배수(dk)·회전(spin) */
    shards: { ch: string; da: number; dk: number; spin: number }[];
}

const smooth = (x: number, a: number, b: number) => {
    const t = Math.min(1, Math.max(0, (x - a) / (b - a)));
    return t * t * (3 - 2 * t);
};

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
        let words: Word[] = [];
        let wordsFor = -1;
        let mono = 'monospace';
        let rows = 0;
        let dpr = 1;
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
            dpr = Math.min(window.devicePixelRatio || 1, 2);
            w = canvas.clientWidth;
            h = canvas.clientHeight;
            canvas.width = Math.round(w * dpr);
            canvas.height = Math.round(h * dpr);
            ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
            mono = readColor(canvas, '--font-mono', 'monospace');
            ctx.font = `${FONT_PX}px ${mono}`;
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

        // 오프닝을 시작할 때 화면에 떠 있던 글자를 낱말 단위로 붙잡는다
        const takeWords = (p: MatrixPull) => {
            // 오프닝 동안은 캔버스를 낮은 해상도로 다시 잡는다 (글자 자리는 CSS 픽셀이라 그대로다)
            dpr = Math.min(window.devicePixelRatio || 1, 2) * OPENING_RESOLUTION;
            canvas.width = Math.round(w * dpr);
            canvas.height = Math.round(h * dpr);
            ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
            ctx.font = `${FONT_PX}px ${mono}`;
            const reach = Math.hypot(w, h) * 0.6;
            words = glyphs.map((g) => {
                const width = ctx.measureText(g.text).width;
                return {
                    text: g.text,
                    x: g.x + width / 2,
                    y: g.y + FONT_PX / 2,
                    width,
                    color: g.color,
                    seen: KEEP ** g.age,
                    spin: (Math.random() < 0.5 ? -1 : 1) * (0.6 + Math.random() * 2.4),
                    angle: Math.random() * Math.PI * 2,
                    dist: (0.25 + Math.random() * 0.85) * reach,
                    v0: Math.random() * h * 0.15,
                    delay: Math.random() * 120 * (1 - g.y / Math.max(1, h)),
                    order: 0,
                    shards: [...g.text].map((ch) => ({
                        ch,
                        da: (Math.random() - 0.5) * 0.9,
                        dk: 0.55 + Math.random() * 0.9,
                        spin: (Math.random() - 0.5) * 8,
                    })),
                };
            });
            // 가운데에서 먼 낱말부터 하나씩 떼어 온다
            const far = (word: Word) => Math.hypot(word.x - p.x, word.y - p.y);
            const ranked = [...words].sort((a, b) => far(b) - far(a));
            ranked.forEach((word, i) => {
                word.order = ranked.length > 1 ? i / (ranked.length - 1) : 1;
            });
            if (p.kind === 'burst') buildAtlas();
            wordsFor = p.start;
        };

        /*
         * 터질 때의 글자 조각은 글자마다 한 번만 그려 둔 그림(atlas)에서 잘라 돌려 찍는다. 조각마다 각도·크기가 달라
         * fillText로 찍으면 매 프레임 수백 개의 글자를 새로 래스터화하느라 화음 직후가 끊긴다.
         * 그림은 낱말을 붙잡을 때(끌어오기 시작) 한 장에 다 그려 두어 화음 순간에는 할 일이 없게 한다
         */
        let atlas: HTMLCanvasElement | null = null;
        const cells = new Map<string, number>();
        const cellHalf = Math.ceil(FONT_PX * SHARD_SCALE);
        const buildAtlas = () => {
            cells.clear();
            for (const word of words) {
                for (const shard of word.shards) if (!cells.has(shard.ch)) cells.set(shard.ch, cells.size);
            }
            const cell = Math.ceil(cellHalf * 2 * dpr);
            atlas = document.createElement('canvas');
            atlas.width = Math.max(1, cells.size) * cell;
            atlas.height = cell;
            const g = atlas.getContext('2d');
            if (!g) return;
            g.scale(dpr, dpr);
            g.font = `600 ${FONT_PX * SHARD_SCALE}px ${mono}`;
            g.textAlign = 'center';
            g.textBaseline = 'middle';
            g.fillStyle = head;
            for (const [ch, i] of cells) g.fillText(ch, (i * cell) / dpr + cellHalf, cellHalf);
        };
        const drawShard = (ch: string, x: number, y: number, turn: number, scale: number) => {
            const i = cells.get(ch);
            if (!atlas || i === undefined) return;
            const cell = atlas.height;
            const cos = Math.cos(turn) * scale * dpr;
            const sin = Math.sin(turn) * scale * dpr;
            ctx.setTransform(cos, sin, -sin, cos, x * dpr, y * dpr);
            ctx.drawImage(atlas, i * cell, 0, cell, cell, -cellHalf, -cellHalf, cellHalf * 2, cellHalf * 2);
        };

        const drawWord = (word: Word, x: number, y: number, angle: number, scale: number, font: string) => {
            ctx.save();
            ctx.translate(x, y);
            ctx.rotate(angle);
            if (scale !== 1) ctx.scale(scale, scale);
            ctx.font = font;
            ctx.fillText(word.text, 0, 0);
            ctx.restore();
        };

        const glow = (x: number, y: number, r: number, alpha: number) => {
            if (alpha <= 0.003) return;
            const g = ctx.createRadialGradient(x, y, 0, x, y, r);
            g.addColorStop(0, head);
            g.addColorStop(1, 'transparent');
            ctx.globalAlpha = alpha;
            ctx.fillStyle = g;
            ctx.fillRect(x - r, y - r, r * 2, r * 2);
        };

        // 정상 기동: 같은 규칙의 곡선으로 끌어모아 먼지로 합치고, 화음에 충격파와 함께 무작위로 흩뿌린다
        const drawBurst = (p: MatrixPull, ms: number) => {
            const f = openingFrame(ms, p.chord);
            const center = { x: p.x, y: p.y };
            const small = Math.min(w, h);
            const font = `${FONT_PX}px ${mono}`;
            if (!f.opened) {
                glow(p.x, p.y, small * (0.1 + 0.2 * f.pull), 0.8 * f.pull);
                glow(p.x, p.y, small * 0.06, 0.9 * f.pull * f.pull);
                const u = ms / p.chord;
                for (const word of words) {
                    const e = wordPull(ms, word.order, p.chord);
                    const at = pullPoint(word, center, e);
                    const angle = word.spin * Math.PI * e;
                    // 떨어져 나오는 순간 살짝 부풀었다가, 가운데로 갈수록 작아진다
                    const lift = 1 + 0.35 * Math.sin(Math.PI * smooth(e, 0, 0.35)) - 0.35 * e;
                    const wordAlpha = (word.seen + (1 - word.seen) * u) * (1 - smooth(e, 0.55, 0.85));
                    if (wordAlpha > 0.02) {
                        ctx.globalAlpha = wordAlpha;
                        ctx.fillStyle = e > 0.02 ? head : word.color;
                        drawWord(word, at.x, at.y, angle, lift, font);
                    }
                    // 가운데 가까이에서 낱말이 먼지로 부서진다 (낱말 길이만큼 늘어선 점이 함께 줄어든다)
                    const dust = smooth(e, 0.4, 0.7) * (1 - smooth(e, 0.97, 1));
                    if (dust > 0.02) {
                        ctx.globalAlpha = dust;
                        ctx.fillStyle = head;
                        const n = Math.max(2, Math.min(6, word.text.length + 1));
                        const span = word.width * (1 - e);
                        for (let i = 0; i < n; i++) {
                            const o = (i / (n - 1) - 0.5) * span;
                            ctx.fillRect(at.x + Math.cos(angle) * o - 0.8, at.y + Math.sin(angle) * o - 0.8, 1.6, 1.6);
                        }
                    }
                }
                return;
            }
            const b = f.burst;
            // 터지면 낱말이 글자 조각으로 다 부서져 저마다 흩어지고, 조각도 곧 먼지가 되어 사라진다
            const shrink = (1.15 - 0.35 * b) / SHARD_SCALE;
            const fade = (1 - b) ** 1.4;
            const glyphAlpha = fade * (1 - smooth(b, 0.2, 0.6));
            const dustAlpha = fade * smooth(b, 0.08, 0.3);
            ctx.strokeStyle = head;
            ctx.fillStyle = head;
            ctx.lineWidth = 1;
            for (const word of words) {
                const seen = Math.max(word.seen, 0.55);
                // 먼지는 낱말마다 길 하나에 모아 한 번에 칠한다
                const dust = dustAlpha > 0.02 ? new Path2D() : null;
                for (const [i, shard] of word.shards.entries()) {
                    const angle = word.angle + shard.da;
                    const dist = word.dist * shard.dk;
                    const at = scatterPoint(center, angle, dist, b);
                    if (i === 0) {
                        const tail = scatterPoint(center, angle, dist, Math.max(0, b - 0.06));
                        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
                        ctx.globalAlpha = fade * 0.35;
                        ctx.beginPath();
                        ctx.moveTo(tail.x, tail.y);
                        ctx.lineTo(at.x, at.y);
                        ctx.stroke();
                    }
                    if (glyphAlpha > 0.02) {
                        ctx.globalAlpha = glyphAlpha * seen;
                        drawShard(shard.ch, at.x, at.y, word.spin * Math.PI + shard.spin * b, shrink);
                    }
                    if (dust) {
                        // 조각 하나가 먼지 셋으로 바스러져 조금씩 벌어진다
                        const spread = 4 + 22 * b;
                        for (let k = 0; k < 3; k++) {
                            const a = angle + shard.spin + (k * Math.PI * 2) / 3;
                            dust.rect(
                                at.x + Math.cos(a) * spread * shard.dk - 0.9,
                                at.y + Math.sin(a) * spread * shard.dk - 0.9,
                                1.8,
                                1.8,
                            );
                        }
                    }
                }
                if (dust) {
                    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
                    ctx.globalAlpha = dustAlpha * seen;
                    ctx.fill(dust);
                }
            }
            ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
            // 충격파 두 겹
            const wave = (ms - p.chord) / 550;
            if (wave < 1) {
                const r = (1 - (1 - wave) ** 3) * Math.hypot(w, h) * 0.7;
                for (const [scale, width, a, color] of [
                    [1, 10, 1, head],
                    [0.9, 18, 0.5, ink],
                ] as const) {
                    ctx.globalAlpha = a * (1 - wave) ** 1.3;
                    ctx.strokeStyle = color;
                    ctx.lineWidth = Math.max(0.5, (1 - wave) * width);
                    ctx.beginPath();
                    ctx.arc(p.x, p.y, r * scale, 0, Math.PI * 2);
                    ctx.stroke();
                }
            }
        };

        // 비정상 기동: 낱말이 제자리에서 후두둑 떨어진다
        const drawFall = (ms: number) => {
            const f = fallFrame(ms);
            const font = `${FONT_PX}px ${mono}`;
            for (const word of words) {
                const t = ms - word.delay;
                const at = fallPoint(word, t, word.v0, h);
                if (at.y > h + 20) continue;
                ctx.globalAlpha = word.seen * f.glyphs;
                ctx.fillStyle = word.color;
                drawWord(word, at.x, at.y, (word.spin * 0.4 * Math.max(0, t)) / 1000, 1, font);
            }
        };

        const drawPull = (p: MatrixPull, now: number) => {
            if (wordsFor !== p.start) takeWords(p);
            ctx.clearRect(0, 0, w, h);
            ctx.textAlign = 'center';
            ctx.textBaseline = 'middle';
            if (p.kind === 'fall') drawFall(now - p.start);
            else drawBurst(p, now - p.start);
            ctx.globalAlpha = 1;
            ctx.textAlign = 'start';
            ctx.textBaseline = 'top';
            ctx.font = `${FONT_PX}px ${mono}`;
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
        // 창이 아니라 캔버스 자리의 크기를 따른다: 자리가 0이었다가 커지거나(전환 화면) 창 크기와 따로 바뀌면
        // 예전 크기로 만든 판이 늘어나 글자가 커다랗게 깨져 보인다
        const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(() => resize()) : null;
        if (observer) observer.observe(canvas);
        else window.addEventListener('resize', resize);
        return () => {
            cancelAnimationFrame(raf);
            observer?.disconnect();
            window.removeEventListener('resize', resize);
        };
    }, []);

    return (
        <div className={className} aria-hidden="true">
            <canvas ref={ref} />
        </div>
    );
}
