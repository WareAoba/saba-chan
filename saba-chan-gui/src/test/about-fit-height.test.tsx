import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { FitHeight } from '../components/Modals/settings/AboutTab';

const rect = (top: number, bottom: number) => ({ top, bottom }) as DOMRect;

describe('설정 › 정보 한 화면 맞추기', () => {
    it('탭이 미끄러져 들어오는 동안(아래로 밀린 채) 재도 줄이지 않는다', () => {
        // 칸 바닥 500, 내용 위 100, 내용 높이 400 → 딱 맞는다. 다만 지금 탭 전체가 16px 아래로 밀려 있다
        const box = document.createElement('div');
        box.className = 'settings-modal-content';
        box.getBoundingClientRect = () => rect(0, 500);
        const slide = document.createElement('div');
        slide.style.transform = 'matrix(1, 0, 0, 1, 0, 16)'; // 브라우저가 계산해 주는 꼴
        box.appendChild(slide);
        document.body.appendChild(box);
        const proto = HTMLElement.prototype;
        const realRect = proto.getBoundingClientRect;
        const realHeight = Object.getOwnPropertyDescriptor(proto, 'offsetHeight');
        proto.getBoundingClientRect = function (this: HTMLElement) {
            return this.classList.contains('fit-height') ? rect(116, 516) : realRect.call(this);
        };
        Object.defineProperty(proto, 'offsetHeight', { configurable: true, get: () => 400 });
        try {
            const { container } = render(
                <FitHeight>
                    <p>about</p>
                </FitHeight>,
                { container: slide },
            );
            expect((container.querySelector('.fit-height') as HTMLElement).style.transform).toBe('');
        } finally {
            proto.getBoundingClientRect = realRect;
            if (realHeight) Object.defineProperty(proto, 'offsetHeight', realHeight);
            box.remove();
        }
    });
});
