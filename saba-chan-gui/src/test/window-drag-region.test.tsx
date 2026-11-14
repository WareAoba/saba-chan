/**
 * 창 끌기 (사용자 11-15): 첫 실행·탭 전환 직후 상단바로 창을 끌 수 없던 문제.
 * CSS `app-region`은 WebView2(비클라이언트 영역 지원)에서 애니메이션이 도는 동안 셸 창 스레드를 1초 넘게 붙잡아
 * 창 끌기(start_dragging)까지 막았다 (PC 실측: app-region만 끄면 탭 전환 멈춤이 사라진다).
 * 끌기는 data-tauri-drag-region 하나로 하고, CSS 영역이 맡던 상단바 빈 자리에도 그 속성을 단다.
 */
import fs from 'node:fs';
import path from 'node:path';
import { render } from '@testing-library/react';
import type { ComponentProps, ComponentType } from 'react';
import { describe, expect, it } from 'vitest';
import { AppBar as AppBarFull } from '../components/shell/AppBar';

const SRC = path.resolve(__dirname, '..');
const AppBar = AppBarFull as unknown as ComponentType<Partial<ComponentProps<typeof AppBarFull>>>;

function cssFiles(dir: string): string[] {
    return fs.readdirSync(dir, { withFileTypes: true }).flatMap((d) => {
        const p = path.join(dir, d.name);
        if (d.isDirectory()) return cssFiles(p);
        return d.name.endsWith('.css') ? [p] : [];
    });
}

describe('창 끌기 영역', () => {
    it('CSS app-region을 쓰지 않는다', () => {
        const offenders = cssFiles(SRC).filter((f) =>
            /(^|[\s;{])(-webkit-)?app-region\s*:/.test(fs.readFileSync(f, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '')),
        );
        expect(offenders.map((f) => path.relative(SRC, f))).toEqual([]);
    });

    it('상단바의 로고·탭 줄·상태 칸 빈 자리를 끌어도 창이 움직인다', () => {
        render(<AppBar page="servers" onNavigate={() => {}} noticeProps={{} as never} daemonProps={{} as never} />);
        for (const sel of [
            '.appbar',
            '.appbar-brand',
            '.appbar-logo',
            '.appbar-wordmark',
            '.appbar-tabs',
            '.appbar-spacer',
            '.appbar-status',
        ]) {
            expect(document.querySelector(sel)?.hasAttribute('data-tauri-drag-region'), sel).toBe(true);
        }
    });
});
