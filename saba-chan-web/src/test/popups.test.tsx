import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Sheet } from '../components/ui';

// 사용자 11-06 #1: 겹치는 창은 화면 기준으로 떠야 한다. 들어오는 움직임이 transform을 남기는 상자 안에 있으면
// position: fixed가 그 상자를 기준으로 놓여 긴 화면의 맨 아래에 붙는다
describe('겹치는 창', () => {
    it('시트는 body 바로 아래에 그린다 (움직이는 화면 상자 안에 두지 않는다)', () => {
        const { container } = render(
            <div className="view">
                <Sheet title="Announce" onClose={() => {}}>
                    <input aria-label="Message" />
                </Sheet>
            </div>,
        );
        expect(container.querySelector('.scrim')).toBeNull();
        const scrim = document.querySelector('.scrim');
        expect(scrim?.parentElement).toBe(document.body);
    });

    it('들어오는 움직임은 끝난 뒤 transform을 남기지 않는다 (fill-mode both 금지)', () => {
        const css = readFileSync(resolve(__dirname, '../styles.css'), 'utf8');
        // transform을 움직이는 keyframes 이름
        const moving = [...css.matchAll(/@keyframes\s+([\w-]+)\s*\{([^}]*\{[^}]*\})*[^}]*\}/g)]
            .filter((m) => m[0].includes('transform'))
            .map((m) => m[1]);
        const offenders = [...css.matchAll(/animation:\s*([\w-]+)[^;]*\bboth\b/g)]
            .map((m) => m[1])
            .filter((name) => moving.includes(name));
        expect(offenders).toEqual([]);
    });
});
