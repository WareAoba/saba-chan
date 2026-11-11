import { act, fireEvent } from '@testing-library/react';

/**
 * Radix 창(Popover·Select·메뉴) 바깥을 누른다. Radix는 여는 누르기와 섞이지 않게 한 틱 뒤부터 바깥을 보고,
 * 누름 뒤 클릭까지 끝나야 닫는다.
 */
export async function clickOutside(target: Element = document.body) {
    await act(() => new Promise((r) => setTimeout(r, 0)));
    fireEvent.pointerDown(target);
    fireEvent.click(target);
}
