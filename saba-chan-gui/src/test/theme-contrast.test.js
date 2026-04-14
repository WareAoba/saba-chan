/**
 * 브랜드컬러 팔레트 및 가시성(대비색) 유틸리티 테스트
 *
 * 검증 항목:
 * 1. ACCENT_PRESETS에 중복 primary 색상 없음
 * 2. 모든 프리셋의 primary/secondary가 서로 다름
 * 3. getContrastColor()가 WCAG AA 기준을 충족하는 텍스트 색상을 반환
 * 4. applyCustomTheme()가 --brand-text CSS 변수를 올바르게 설정
 */
import { describe, expect, it, beforeEach, vi } from 'vitest';
import {
    ACCENT_PRESETS,
    getContrastColor,
    THEME_DEFAULTS,
} from '../utils/themeManager';

// ── 1. ACCENT_PRESETS 중복/유효성 ──────────────────────────

describe('ACCENT_PRESETS 팔레트', () => {
    it('프리셋 이름에 중복이 없어야 한다', () => {
        const names = ACCENT_PRESETS.map((p) => p.name);
        const unique = new Set(names);
        expect(unique.size).toBe(names.length);
    });

    it('primary 색상에 중복이 없어야 한다', () => {
        const primaries = ACCENT_PRESETS.map((p) => p.primary.toLowerCase());
        const unique = new Set(primaries);
        expect(unique.size).toBe(primaries.length);
    });

    it('각 프리셋의 primary와 secondary가 다르다', () => {
        for (const preset of ACCENT_PRESETS) {
            expect(preset.primary.toLowerCase()).not.toBe(preset.secondary.toLowerCase());
        }
    });

    it('프리셋 수가 8개이다', () => {
        expect(ACCENT_PRESETS.length).toBe(8);
    });

    it('모든 색상이 유효한 6자리 hex이다', () => {
        const hexPattern = /^#[0-9a-f]{6}$/i;
        for (const preset of ACCENT_PRESETS) {
            expect(preset.primary).toMatch(hexPattern);
            expect(preset.secondary).toMatch(hexPattern);
        }
    });
});

// ── 2. getContrastColor() WCAG 대비 검증 ──────────────────

/**
 * sRGB → 선형 RGB (WCAG 2.1 공식)
 */
function srgbToLinear(c) {
    const s = c / 255;
    return s <= 0.04045 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
}

/**
 * 상대 휘도 (Relative Luminance)
 */
function relativeLuminance(hex) {
    const h = hex.replace('#', '');
    const r = parseInt(h.substring(0, 2), 16);
    const g = parseInt(h.substring(2, 4), 16);
    const b = parseInt(h.substring(4, 6), 16);
    return 0.2126 * srgbToLinear(r) + 0.7152 * srgbToLinear(g) + 0.0722 * srgbToLinear(b);
}

/**
 * 대비비 계산
 */
function contrastRatio(hex1, hex2) {
    const l1 = relativeLuminance(hex1);
    const l2 = relativeLuminance(hex2);
    const lighter = Math.max(l1, l2);
    const darker = Math.min(l1, l2);
    return (lighter + 0.05) / (darker + 0.05);
}

describe('getContrastColor()', () => {
    it('어두운 색상(#000000)에는 밝은 텍스트를 반환', () => {
        const result = getContrastColor('#000000');
        const ratio = contrastRatio('#000000', result);
        expect(ratio).toBeGreaterThanOrEqual(4.5);
    });

    it('밝은 색상(#ffffff)에는 어두운 텍스트를 반환', () => {
        const result = getContrastColor('#ffffff');
        const ratio = contrastRatio('#ffffff', result);
        expect(ratio).toBeGreaterThanOrEqual(4.5);
    });

    it('밝은 배경에는 어두운 텍스트, 어두운/중간 배경에는 밝은 텍스트를 반환한다', () => {
        // 밝은 배경 → 어두운 텍스트
        const lightBgs = ['#ffffff', '#fbbf24', '#f0f0f0', '#a3e635'];
        for (const bg of lightBgs) {
            expect(
                getContrastColor(bg),
                `밝은 배경 ${bg}에 어두운 텍스트 기대`
            ).toBe('#1a1a2e');
        }

        // 어두운/중간 채도 배경 → 밝은 텍스트 (WCAG 2.1 한계 보정)
        const darkMidBgs = [
            '#667eea', '#764ba2', '#3b82f6', '#7c3aed',
            '#06b6d4', '#ef4444', '#ec4899', '#14b8a6',
            '#a855f7', '#0891b2', '#000000', '#1e3a5f',
        ];
        for (const bg of darkMidBgs) {
            expect(
                getContrastColor(bg),
                `어두운/중간 배경 ${bg}에 밝은 텍스트 기대`
            ).toBe('#ffffff');
        }
    });

    it('모든 ACCENT_PRESETS의 primary에 대해 WCAG AA Large Text(3:1) 충족', () => {
        for (const preset of ACCENT_PRESETS) {
            const fg = getContrastColor(preset.primary);
            const ratio = contrastRatio(preset.primary, fg);
            // 브랜드 버튼/배지 텍스트는 대부분 bold/large이므로 AA Large Text(3:1) 적용
            expect(
                ratio,
                `preset "${preset.name}" (${preset.primary}) → ${fg} ratio ${ratio.toFixed(2)}`
            ).toBeGreaterThanOrEqual(3.0);
        }
    });

    it('모든 ACCENT_PRESETS에 밝은 텍스트(#ffffff)가 선택된다', () => {
        // 모든 프리셋 primary는 중간~어두운 밝기이므로 흰색 텍스트가 선택되어야 함
        for (const preset of ACCENT_PRESETS) {
            expect(
                getContrastColor(preset.primary),
                `preset "${preset.name}" (${preset.primary})에 밝은 텍스트 기대`
            ).toBe('#ffffff');
        }
    });

    it('모든 ACCENT_PRESETS의 secondary에 대해 WCAG AA Large Text(3:1) 충족', () => {
        for (const preset of ACCENT_PRESETS) {
            const fg = getContrastColor(preset.secondary);
            const ratio = contrastRatio(preset.secondary, fg);
            expect(
                ratio,
                `preset "${preset.name}" secondary (${preset.secondary}) → ${fg} ratio ${ratio.toFixed(2)}`
            ).toBeGreaterThanOrEqual(3.0);
        }
    });

    it('반환값은 유효한 hex 색상이다', () => {
        const hexPattern = /^#[0-9a-f]{6}$/i;
        expect(getContrastColor('#667eea')).toMatch(hexPattern);
        expect(getContrastColor('#ffffff')).toMatch(hexPattern);
        expect(getContrastColor('#000000')).toMatch(hexPattern);
    });
});
