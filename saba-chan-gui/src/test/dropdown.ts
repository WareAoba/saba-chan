import { fireEvent, within } from '@testing-library/react';

/** CustomDropdown의 팝업 목록 (트리거를 눌러 연다) */
export function openDropdown(trigger: HTMLElement) {
    fireEvent.click(trigger);
    return document.getElementById(trigger.getAttribute('aria-controls') ?? '') as HTMLElement;
}

/** CustomDropdown에서 이름으로 항목을 고른다 */
export function pickOption(trigger: HTMLElement, name: string | RegExp) {
    fireEvent.click(within(openDropdown(trigger)).getByRole('option', { name }));
}
