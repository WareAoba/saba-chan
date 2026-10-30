import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

// 콘솔 입력줄: 포커스는 둥근 바깥 칸(.term-input.focused)의 링으로만 보인다.
// base.css의 전역 :focus-visible 외곽선이 안쪽 input에 각진 사각형으로 그려지면 안 된다.
// base.css는 App(shell.css)보다 늦게 import되어 같은 명시도 규칙을 이기므로, :focus-visible로 직접 꺼야 한다.
describe('콘솔 입력줄 포커스', () => {
    it('입력 칸은 :focus-visible 외곽선을 끈다', () => {
        const css = readFileSync(join(__dirname, '../styles/shell.css'), 'utf8');
        expect(css).toMatch(/\.term-field:focus-visible\s*\{[^}]*outline:\s*none/);
    });
});
