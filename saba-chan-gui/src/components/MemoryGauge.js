import React from 'react';

/**
 * MemoryGauge — 아날로그 계기판 스타일 메모리/리소스 게이지
 *
 * 눈금(대/소), 바늘, 글로우 효과가 있는 자동차 계기판 느낌.
 *
 * @param {number}  percent  - 사용 퍼센트 (0~100)
 * @param {string}  [usage]  - 사용량 텍스트 (예: "256MiB / 4GiB")
 * @param {number}  [size]   - 게이지 크기 (기본 100)
 * @param {boolean} [compact]- true 이면 헤더용 미니 게이지 (바늘 + 눈금만, 텍스트 없음)
 */
export function MemoryGauge({ percent = 0, usage, size = 100, compact = false }) {
    const pct = Math.min(100, Math.max(0, percent));

    // ── 레이아웃 ───────────────────────────────────────
    // 240° 스윕 (좌측 하단 → 우측 하단)
    const sweepDeg = 240;
    const startDeg = (180 + (360 - sweepDeg) / 2);  // 150°
    const toRad = (d) => (d * Math.PI) / 180;

    const strokeW  = compact ? size * 0.08 : size * 0.055;
    const pad      = compact ? 2 : 4;
    const radius   = (size / 2) - strokeW - pad;
    const cx       = size / 2;
    const cy       = size / 2;

    // 각도 헬퍼 (시계 방향, 12시=0°)
    const angleAt = (p) => startDeg + (sweepDeg * p) / 100;
    const polar   = (deg, r) => ({
        x: cx + r * Math.cos(toRad(deg - 90)),
        y: cy + r * Math.sin(toRad(deg - 90)),
    });

    // ── 색상 ──────────────────────────────────────────
    const color = pct < 60
        ? '#4caf50'
        : pct < 85
            ? '#ff9800'
            : '#f44336';

    // ── 호(arc) 경로 ─────────────────────────────────
    const arcPath = (from, to, r) => {
        const s = polar(from, r);
        const e = polar(to, r);
        const large = (to - from) > 180 ? 1 : 0;
        return `M ${s.x} ${s.y} A ${r} ${r} 0 ${large} 1 ${e.x} ${e.y}`;
    };

    const bgArc  = arcPath(angleAt(0), angleAt(100), radius);
    const valArc = pct > 0 ? arcPath(angleAt(0), angleAt(pct), radius) : '';

    // ── 눈금 ──────────────────────────────────────────
    const ticks = [];
    const majorCount = compact ? 5 : 10; // 0,10,20…100  or 0,20,40…100
    const minorPer   = compact ? 1 : 4;  // 대눈금 사이 소눈금 수
    const majorLen   = compact ? size * 0.10 : size * 0.11;
    const minorLen   = compact ? size * 0.05 : size * 0.055;
    const majorW     = compact ? 1 : 1.5;
    const minorW     = 0.7;

    for (let i = 0; i <= majorCount; i++) {
        const p = (i / majorCount) * 100;
        const deg = angleAt(p);
        const outer = polar(deg, radius - strokeW / 2 - 1);
        const inner = polar(deg, radius - strokeW / 2 - 1 - majorLen);
        // 위험 구간(≥80%) 눈금은 빨갛게
        const tickCol = p >= 80
            ? 'rgba(244,67,54,0.7)'
            : 'rgba(255,255,255,0.35)';
        ticks.push(
            <line key={`M${i}`}
                x1={outer.x} y1={outer.y} x2={inner.x} y2={inner.y}
                stroke={tickCol} strokeWidth={majorW} strokeLinecap="round" />
        );
        // 소눈금
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

    // ── 바늘 (needle) ─────────────────────────────────
    const needleDeg = angleAt(pct);
    const needleLen = radius - strokeW / 2 - majorLen - (compact ? 1 : 4);
    const needleTip = polar(needleDeg, needleLen);
    // 바늘 뒤쪽 약간 돌출
    const needleTail = polar(needleDeg + 180, compact ? 3 : size * 0.06);
    const needleW = compact ? 1.2 : 2;

    // 중심 피벗 원
    const pivotR = compact ? 2 : size * 0.04;

    // ── 숫자 라벨 (full 모드만) ───────────────────────
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

    // ── 사용량 텍스트 파싱 ─────────────────────────────
    let usedLabel = '';
    let totalLabel = '';
    if (usage) {
        const parts = usage.split('/').map(s => s.trim());
        if (parts.length === 2) {
            usedLabel = parts[0].replace('iB', '').replace('B', '');
            totalLabel = parts[1].replace('iB', '').replace('B', '');
        }
    }

    // ── compact 모드 ──────────────────────────────────
    if (compact) {
        return (
            <div className="memory-gauge memory-gauge-compact"
                style={{ width: size, height: size, position: 'relative' }}>
                <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
                    {/* 배경 호 */}
                    <path d={bgArc} fill="none"
                        stroke="rgba(255,255,255,0.10)" strokeWidth={strokeW} strokeLinecap="round" />
                    {/* 값 호 */}
                    {pct > 0 && (
                        <path d={valArc} fill="none"
                            stroke={color} strokeWidth={strokeW} strokeLinecap="round"
                            style={{ transition: 'all 0.5s ease' }} />
                    )}
                    {/* 눈금 */}
                    {ticks}
                    {/* 바늘 */}
                    <line x1={needleTail.x} y1={needleTail.y} x2={needleTip.x} y2={needleTip.y}
                        stroke="rgba(255,255,255,0.85)" strokeWidth={needleW} strokeLinecap="round"
                        style={{ transition: 'all 0.5s ease' }} />
                    <circle cx={cx} cy={cy} r={pivotR} fill="rgba(255,255,255,0.6)" />
                </svg>
            </div>
        );
    }

    // ── full 모드 ─────────────────────────────────────
    const pctFontSize = Math.max(12, size * 0.18);
    const subFontSize = Math.max(8, size * 0.085);
    // 퍼센트 텍스트 위치: 중심 살짝 아래
    const pctY = cy + size * 0.16;
    const subY = pctY + pctFontSize * 0.85;
    // 단위 라벨 ("MEM") 위치
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
                {/* 배경 호 */}
                <path d={bgArc} fill="none"
                    stroke="rgba(255,255,255,0.08)" strokeWidth={strokeW} strokeLinecap="round" />
                {/* 값 호 (글로우) */}
                {pct > 0 && (
                    <path d={valArc} fill="none"
                        stroke={color} strokeWidth={strokeW * 2.5} strokeLinecap="round"
                        opacity="0.2" filter={`url(#${uid})`}
                        style={{ transition: 'all 0.6s ease' }} />
                )}
                {/* 값 호 (실선) */}
                {pct > 0 && (
                    <path d={valArc} fill="none"
                        stroke={color} strokeWidth={strokeW} strokeLinecap="round"
                        style={{ transition: 'all 0.6s ease' }} />
                )}
                {/* 눈금 */}
                {ticks}
                {/* 숫자 라벨 */}
                {labels}
                {/* 단위 표기 */}
                <text x={cx} y={unitY}
                    textAnchor="middle" dominantBaseline="central"
                    fill="rgba(255,255,255,0.25)"
                    fontSize={subFontSize} fontWeight="600" letterSpacing="1.5"
                    fontFamily="inherit">
                    MEM
                </text>
                {/* 바늘 (글로우) */}
                <line x1={needleTail.x} y1={needleTail.y} x2={needleTip.x} y2={needleTip.y}
                    stroke={color} strokeWidth={needleW + 3} strokeLinecap="round"
                    opacity="0.25" filter={`url(#${uid})`}
                    style={{ transition: 'all 0.6s ease' }} />
                {/* 바늘 (실선) */}
                <line x1={needleTail.x} y1={needleTail.y} x2={needleTip.x} y2={needleTip.y}
                    stroke="rgba(255,255,255,0.92)" strokeWidth={needleW} strokeLinecap="round"
                    style={{ transition: 'all 0.6s ease' }} />
                {/* 피벗 */}
                <circle cx={cx} cy={cy} r={pivotR + 1} fill="rgba(255,255,255,0.08)" />
                <circle cx={cx} cy={cy} r={pivotR} fill="rgba(255,255,255,0.55)" />
                <circle cx={cx} cy={cy} r={pivotR * 0.45} fill={color}
                    style={{ transition: 'fill 0.6s ease' }} />
                {/* 퍼센트 */}
                <text x={cx} y={pctY}
                    textAnchor="middle" dominantBaseline="central"
                    fill={color}
                    fontSize={pctFontSize} fontWeight="700"
                    fontFamily="inherit"
                    style={{ transition: 'fill 0.6s ease' }}>
                    {Math.round(pct)}%
                </text>
                {/* 사용량 서브 라벨 */}
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
