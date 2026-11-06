import { fireEvent } from '@testing-library/react';

/** DateField에서 날짜를 고른다 (달력을 열고 그 달까지 넘긴 뒤 누른다) */
export function pickDate(trigger: HTMLElement, date: string) {
    fireEvent.click(trigger);
    const panel = document.getElementById(trigger.getAttribute('aria-controls') ?? '') as HTMLElement;
    for (let i = 0; i < 24 && !panel.querySelector(`[data-date="${date}"]`); i++) {
        const [prev, next] = panel.querySelectorAll('.dtp-head button');
        const first = (panel.querySelector('[data-date]') as HTMLElement).dataset.date as string;
        fireEvent.click(date < first ? (prev as Element) : (next as Element));
    }
    fireEvent.click(panel.querySelector(`[data-date="${date}"]`) as Element);
}

/** TimeField에서 "HH:MM"을 고른다 */
export function pickTime(trigger: HTMLElement, time: string) {
    const [h, m] = time.split(':');
    fireEvent.click(trigger);
    const panel = document.getElementById(trigger.getAttribute('aria-controls') ?? '') as HTMLElement;
    fireEvent.click(panel.querySelector(`[data-hour="${h}"]`) as Element);
    fireEvent.click(panel.querySelector(`[data-minute="${m}"]`) as Element);
}
