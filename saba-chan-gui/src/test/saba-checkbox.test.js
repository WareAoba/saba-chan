import { fireEvent, render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { SabaCheckbox } from '../components/ui/SabaUI';

describe('SabaCheckbox', () => {
    it('체크 표시는 상자 정중앙에 놓인 SVG 하나다', () => {
        const { container } = render(<SabaCheckbox checked onChange={() => {}} />);
        const path = container.querySelector('.saba-checkbox-box svg path');
        expect(path).not.toBeNull();
        // 16×16 상자에서 체크가 차지하는 범위(가로 4~12, 세로 5~11)의 중심이 (8, 8)
        const nums = path.getAttribute('d').match(/-?\d+(\.\d+)?/g).map(Number);
        const xs = nums.filter((_, i) => i % 2 === 0);
        const ys = nums.filter((_, i) => i % 2 === 1);
        expect((Math.min(...xs) + Math.max(...xs)) / 2).toBe(8);
        expect((Math.min(...ys) + Math.max(...ys)) / 2).toBe(8);
    });

    it('글자를 함께 주면 글자를 눌러도 바뀐다', () => {
        const onChange = vi.fn();
        const { getByText } = render(
            <SabaCheckbox checked={false} onChange={onChange}>
                약관에 동의합니다
            </SabaCheckbox>,
        );
        const text = getByText('약관에 동의합니다');
        expect(text.className).toBe('saba-checkbox-label');
        fireEvent.click(text);
        expect(onChange).toHaveBeenCalledWith(true);
    });
});
