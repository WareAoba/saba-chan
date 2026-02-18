/**
 * MemoryGauge — 아날로그 계기판 스타일 메모리/리소스 게이지
 * Docker 익스텐션용 독립 복사본 (호스트 앱 의존성 없음)
 *
 * @param {number}  percent  - 사용 퍼센트 (0~100)
 * @param {string}  [usage]  - 사용량 텍스트 (예: "256MiB / 4GiB")
 * @param {number}  [size]   - 게이지 크기 (기본 100)
 * @param {boolean} [compact]- true 이면 헤더용 미니 게이지 (바늘 + 눈금만, 텍스트 없음)
 */
import React from 'react';

export function MemoryGauge({ percent = 0, usage, size = 100, compact = false }) {
    const pct = Math.min(100, Math.max(0, percent));

    const sweepDeg = 240;
    const startDeg = (180 + (360 - sweepDeg) / 2);
    const toRad = (d) => (d * Math.PI) / 180;

    const strokeW  = compact ? size * 0.08 : size * 0.055;
    const pad      = compact ? 2 : 4;
    const radius   = (size / 2) - strokeW - pad;
    const cx       = size / 2;
    const cy       = size / 2;

    const angleAt = (p) => startDeg + (sweepDeg * p) / 100;
    const polar   = (deg, r) => ({
        x: cx + r * Math.cos(toRad(deg - 90)),
        y: cy + r * Math.sin(toRad(deg - 90)),
    });

    const color = pct < 60 ? '#4caf50' : pct < 85 ? '#ff9800' : '#f44336';

    const arcPath = (from, to, r) => {
        const s = polar(from, r);
        const e = polar(to, r);
        const large = (to - from) > 180 ? 1 : 0;
        return `M ${s.x} ${s.y} A ${r} ${r} 0 ${large} 1 ${e.x} ${e.y}`;
    };

    const bgArc  = arcPath(angleAt(0), angleAt(100), radius);
    const valArc = pct > 0 ? arcPath(angleAt(0), angleAt(pct), radius) : '';

    const ticks = [];
    const majorCount = compact ? 5 : 10;
    const minorPer   = compact ? 1 : 4;
    const majorLen   = compact ? size * 0.10 : size * 0.11;
    const minorLen   = compact ? size * 0.05 : size * 0.055;
    const majorW     = compact ? 1 : 1.5;
    const minorW     = 0.7;

    for (let i = 0; i <= majorCount; i++) {
        const p = (i / majorCount) * 100;
        const deg = angleAt(p);
        const outer = polar(deg, radius - strokeW / 2 - 1);
        const inner = polar(deg, radius - strokeW / 2 - 1 - majorLen);
        const tickCol = p >= 80 ? 'rgba(244,67,54,0.7)' : 'rgba(255,255,255,0.35)';
        ticks.push(
            <line key={`M${i}`}
                x1={outer.x} y1={outer.y} x2={inner.x} y2={inner.y}
                stroke={tickCol} strokeWidth={majorW} strokeLinecap="round" />
        );
        if (i < majorCount) {
            for (let j = 1; j <= minorPer; j++) {
                const mp = p + (j / (minorPer + 1)) * (100 / majorCount);
                const md = angleAt(mp);
                const mo = polar(md, radius - strokeW / 2 - 1);
                const mi = polar(md, radius - strokeW / 2 - 1 - minorLen);
                ticks.push(
                    <line key={`m${i}_${j}`}
                        x1={mo.x} y1={mo.y} x2={mi.x} y2={mi.y}
                        stroke="rgba(255,255,255,0.15)" strokeWidth={minorW} strokeLinecap="round" />
                );
            }
        }
    }

    const needleDeg = angleAt(pct);
    const needleLen = radius - strokeW / 2 - majorLen - (compact ? 1 : 4);
    const needleTip = polar(needleDeg, needleLen);
    const needleTail = polar(needleDeg + 180, compact ? 3 : size * 0.06);
    const needleW = compact ? 1.2 : 2;
    const pivotR = compact ? 2 : size * 0.04;

    const labels = [];
    if (!compact) {
        const labelR = radius - strokeW / 2 - majorLen - size * 0.10;
        const labelSize = Math.max(7, size * 0.095);
        for (let i = 0; i <= majorCount; i++) {
            const p = (i / majorCount) * 100;
            const deg = angleAt(p);
            const pos = polar(deg, labelR);
            labels.push(
                <text key={`L${i}`}
                    x={pos.x} y={pos.y}
                    textAnchor="middle" dominantBaseline="central"
                    fill={p >= 80 ? 'rgba(244,67,54,0.8)' : 'rgba(255,255,255,0.45)'}
                    fontSize={labelSize}
                    fontWeight={p % 20 === 0 ? '600' : '400'}
                    fontFamily="inherit"
                >
                    {Math.round(p)}
                </text>
            );
        }
    }

    let usedLabel = '';
    let totalLabel = '';
    if (usage) {
        const parts = usage.split('/').map(s => s.trim());
        if (parts.length === 2) {
            usedLabel = parts[0].replace('iB', '').replace('B', '');
            totalLabel = parts[1].replace('iB', '').replace('B', '');
        }
    }

    if (compact) {
        return (
            <div className="memory-gauge memory-gauge-compact"
                style={{ width: size, height: size, position: 'relative', flexShrink: 0, marginLeft: '6px', marginRight: '2px', opacity: 0.9 }}>
                <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
                    <path d={bgArc} fill="none"
                        stroke="rgba(255,255,255,0.10)" strokeWidth={strokeW} strokeLinecap="round" />
                    {pct > 0 && (
                        <path d={valArc} fill="none"
                            stroke={color} strokeWidth={strokeW} strokeLinecap="round"
                            style={{ transition: 'all 0.5s ease' }} />
                    )}
                    {ticks}
                    <line x1={needleTail.x} y1={needleTail.y} x2={needleTip.x} y2={needleTip.y}
                        stroke="rgba(255,255,255,0.85)" strokeWidth={needleW} strokeLinecap="round"
                        style={{ transition: 'all 0.5s ease' }} />
                    <circle cx={cx} cy={cy} r={pivotR} fill="rgba(255,255,255,0.6)" />
                </svg>
            </div>
        );
    }

    const pctFontSize = Math.max(12, size * 0.18);
    const subFontSize = Math.max(8, size * 0.085);
    const pctY = cy + size * 0.16;
    const subY = pctY + pctFontSize * 0.85;
    const unitY = cy - size * 0.05;
    const uid = `gauge-glow-${Math.random().toString(36).slice(2, 8)}`;

    return (
        <div className="memory-gauge" style={{ width: size, height: size, position: 'relative' }}>
            <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
                <defs>
                    <filter id={uid} x="-20%" y="-20%" width="140%" height="140%">
                        <feGaussianBlur in="SourceGraphic" stdDeviation="2.5" />
                    </filter>
                </defs>
                <path d={bgArc} fill="none"
                    stroke="rgba(255,255,255,0.08)" strokeWidth={strokeW} strokeLinecap="round" />
                {pct > 0 && (
                    <path d={valArc} fill="none"
                        stroke={color} strokeWidth={strokeW * 2.5} strokeLinecap="round"
                        opacity="0.2" filter={`url(#${uid})`}
                        style={{ transition: 'all 0.6s ease' }} />
                )}
                {pct > 0 && (
                    <path d={valArc} fill="none"
                        stroke={color} strokeWidth={strokeW} strokeLinecap="round"
                        style={{ transition: 'all 0.6s ease' }} />
                )}
                {ticks}
                {labels}
                <text x={cx} y={unitY}
                    textAnchor="middle" dominantBaseline="central"
                    fill="rgba(255,255,255,0.25)"
                    fontSize={subFontSize} fontWeight="600" letterSpacing="1.5"
                    fontFamily="inherit">
                    MEM
                </text>
                <line x1={needleTail.x} y1={needleTail.y} x2={needleTip.x} y2={needleTip.y}
                    stroke={color} strokeWidth={needleW + 3} strokeLinecap="round"
                    opacity="0.25" filter={`url(#${uid})`}
                    style={{ transition: 'all 0.6s ease' }} />
                <line x1={needleTail.x} y1={needleTail.y} x2={needleTip.x} y2={needleTip.y}
                    stroke="rgba(255,255,255,0.92)" strokeWidth={needleW} strokeLinecap="round"
                    style={{ transition: 'all 0.6s ease' }} />
                <circle cx={cx} cy={cy} r={pivotR + 1} fill="rgba(255,255,255,0.08)" />
                <circle cx={cx} cy={cy} r={pivotR} fill="rgba(255,255,255,0.55)" />
                <circle cx={cx} cy={cy} r={pivotR * 0.45} fill={color}
                    style={{ transition: 'fill 0.6s ease' }} />
                <text x={cx} y={pctY}
                    textAnchor="middle" dominantBaseline="central"
                    fill={color}
                    fontSize={pctFontSize} fontWeight="700"
                    fontFamily="inherit"
                    style={{ transition: 'fill 0.6s ease' }}>
                    {Math.round(pct)}%
                </text>
                {usedLabel && (
                    <text x={cx} y={subY}
                        textAnchor="middle" dominantBaseline="central"
                        fill="rgba(255,255,255,0.4)"
                        fontSize={subFontSize}
                        fontFamily="inherit">
                        {usedLabel} / {totalLabel}
                    </text>
                )}
            </svg>
        </div>
    );
}

export default MemoryGauge;
