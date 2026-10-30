import { fireEvent } from '@testing-library/react';

/** DateField에서 날짜를 고른다 (달력을 열고 그 달까지 넘긴 뒤 누른다) */
export function pickDate(trigger, date) {
    fireEvent.click(trigger);
    const panel = document.getElementById(trigger.getAttribute('aria-controls'));
    for (let i = 0; i < 24 && !panel.querySelector(`[data-date="${date}"]`); i++) {
        const [prev, next] = panel.querySelectorAll('.dtp-head button');
        const first = panel.querySelector('[data-date]').dataset.date;
        fireEvent.click(date < first ? prev : next);
    }
    fireEvent.click(panel.querySelector(`[data-date="${date}"]`));
}

/** TimeField에서 "HH:MM"을 고른다 */
export function pickTime(trigger, time) {
    const [h, m] = time.split(':');
    fireEvent.click(trigger);
    const panel = document.getElementById(trigger.getAttribute('aria-controls'));
    fireEvent.click(panel.querySelector(`[data-hour="${h}"]`));
    fireEvent.click(panel.querySelector(`[data-minute="${m}"]`));
}
