import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import CustomDropdown from '../components/CustomDropdown/CustomDropdown';
import { openDropdown, pickOption } from './dropdown';

const options = [
    { value: 'a', label: 'Apple' },
    { value: 'b', label: 'Banana' },
    { value: 3, label: 'Cherry', disabled: true },
];

// 동작(키보드·포커스·글자 찾기·화면 밖 회피)은 Radix Select가 맡는다. 여기서는 우리 계약만 본다
describe('CustomDropdown', () => {
    it('닫힌 칸에서 글자를 치면 그 글자로 시작하는 항목을 고른다', () => {
        const onChange = vi.fn();
        render(<CustomDropdown value="a" onChange={onChange} options={options} ariaLabel="fruit" />);
        fireEvent.keyDown(screen.getByRole('combobox', { name: 'fruit' }), { key: 'b' });
        expect(onChange).toHaveBeenCalledWith('b');
    });

    it('항목을 고르면 원래 값 종류(숫자·불리언)로 돌려준다', () => {
        const onChange = vi.fn();
        render(
            <CustomDropdown
                value={false}
                onChange={onChange}
                options={[
                    { value: false, label: 'Off' },
                    { value: 10, label: 'Ten' },
                ]}
                ariaLabel="n"
            />,
        );
        pickOption(screen.getByRole('combobox', { name: 'n' }), 'Ten');
        expect(onChange).toHaveBeenCalledWith(10);
    });

    it('막힌 항목은 고를 수 없다', () => {
        const onChange = vi.fn();
        render(<CustomDropdown value="a" onChange={onChange} options={options} ariaLabel="fruit" />);
        const list = openDropdown(screen.getByRole('combobox', { name: 'fruit' }));
        expect(list.querySelector('[data-disabled]')?.textContent).toContain('Cherry');
    });
});
