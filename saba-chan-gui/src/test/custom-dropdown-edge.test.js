import { render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import CustomDropdown from '../components/CustomDropdown/CustomDropdown';
import { openDropdown } from './dropdown';

const options = [
    { value: 'a', label: 'A' },
    { value: 'b', label: '아주 긴 이름의 항목' },
];

function at(left, width) {
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({
        left,
        right: left + width,
        top: 100,
        bottom: 136,
        width,
        height: 36,
    });
}

describe('CustomDropdown 메뉴 자리', () => {
    afterEach(() => vi.restoreAllMocks());

    it('화면 오른쪽에 있는 드롭다운은 메뉴를 오른쪽 끝에 맞춰 화면 밖으로 나가지 않게 연다', () => {
        window.innerWidth = 375;
        at(240, 120);
        render(<CustomDropdown value="a" onChange={() => {}} options={options} ariaLabel="level" />);
        const menu = openDropdown(screen.getByRole('combobox', { name: 'level' })).closest('.custom-dropdown-menu');
        expect(menu.style.right).toBe('15px');
        expect(menu.style.left).toBe('');
    });

    it('왼쪽에 있으면 지금처럼 왼쪽 끝에 맞춘다', () => {
        window.innerWidth = 375;
        at(16, 120);
        render(<CustomDropdown value="a" onChange={() => {}} options={options} ariaLabel="level" />);
        const menu = openDropdown(screen.getByRole('combobox', { name: 'level' })).closest('.custom-dropdown-menu');
        expect(menu.style.left).toBe('16px');
    });
});
