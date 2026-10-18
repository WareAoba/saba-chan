// OKLCH ↔ sRGB hex 변환 (Björn Ottosson의 OKLab 행렬)
// 컬러 피커가 쓰고, 테마 저장 값은 계속 hex로 둔다

export const MAX_CHROMA = 0.37;

const lin = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const gam = (c) => (c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055);

/** OKLCH → 선형이 아닌 sRGB [r,g,b] (0~1, 범위를 넘을 수 있다) */
export function oklchToRgb(L, C, h) {
    const hr = (h * Math.PI) / 180;
    const a = C * Math.cos(hr);
    const b = C * Math.sin(hr);
    const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
    const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
    const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
    return [
        gam(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
        gam(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
        gam(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
    ];
}

const EPS = 0.0005;
export const inGamut = (rgb) => rgb.every((v) => v >= -EPS && v <= 1 + EPS);

/** 이 밝기·색상에서 sRGB 안에 드는 가장 큰 채도 */
export function maxChroma(L, h) {
    let lo = 0;
    let hi = MAX_CHROMA;
    for (let i = 0; i < 16; i++) {
        const mid = (lo + hi) / 2;
        if (inGamut(oklchToRgb(L, mid, h))) lo = mid;
        else hi = mid;
    }
    return lo;
}

const clamp01 = (v) => Math.min(1, Math.max(0, v));
const hex2 = (v) =>
    Math.round(clamp01(v) * 255)
        .toString(16)
        .padStart(2, '0');

export function rgbToHex([r, g, b]) {
    return `#${hex2(r)}${hex2(g)}${hex2(b)}`;
}

/** 범위를 넘으면 채도를 줄여 맞춘 hex */
export function oklchToHex(L, C, h) {
    const c = Math.min(C, maxChroma(L, h));
    return rgbToHex(oklchToRgb(L, c, h));
}

export function parseHex(hex) {
    const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(String(hex || '').trim());
    if (!m) return null;
    let v = m[1];
    if (v.length === 3) v = [...v].map((c) => c + c).join('');
    return [0, 2, 4].map((i) => parseInt(v.slice(i, i + 2), 16) / 255);
}

/** hex → { l, c, h } (h는 무채색이면 null) */
export function hexToOklch(hex) {
    const rgb = parseHex(hex);
    if (!rgb) return null;
    const [r, g, b] = rgb.map(lin);
    const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
    const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
    const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
    const L = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
    const A = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
    const B = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
    const C = Math.sqrt(A * A + B * B);
    const h = C < 0.002 ? null : ((Math.atan2(B, A) * 180) / Math.PI + 360) % 360;
    return { l: L, c: C, h };
}
