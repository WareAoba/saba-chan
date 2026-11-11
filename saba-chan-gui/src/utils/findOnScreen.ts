/**
 * 화면을 연 뒤 이름이 `label`인 설정 칸을 찾아 보이게 하고 잠깐 반짝인다 (명령 팔레트의 설정 칸 바로 가기).
 * 서버 설정(`.ss-row` › `.ss-name`)과 앱 설정(`.setting-item` › `.setting-title`)의 이름표 글로 찾는다.
 * 화면이 그려지기를 잠깐(최대 2초) 기다리고, 끝내 없으면 아무것도 하지 않는다 (화면은 이미 열려 있다).
 */
const ROWS = '.ss-row, .setting-item';
const NAMES = '.ss-name, .setting-title';
const FOUND_CLASS = 'palette-found';

function findRow(label: string) {
    const want = label.trim();
    for (const row of Array.from(document.querySelectorAll<HTMLElement>(ROWS))) {
        // 접힌 묶음(hidden, Ctrl+F가 그려 두고 숨긴 고급 설정) 안의 칸은 펼쳐질 때까지 기다린다
        if (row.closest('[hidden]')) continue;
        const name = row.querySelector(NAMES)?.textContent?.trim() ?? '';
        if (name.startsWith(want)) return row;
    }
    return null;
}

export function flashSetting(label: string, { timeout = 2000, interval = 100 } = {}) {
    const started = Date.now();
    const tick = () => {
        const row = findRow(label);
        if (!row) {
            if (Date.now() - started < timeout) setTimeout(tick, interval);
            return;
        }
        row.scrollIntoView?.({ block: 'center', behavior: 'smooth' });
        row.classList.remove(FOUND_CLASS);
        void row.offsetWidth; // 다시 찾았을 때도 처음부터 반짝인다
        row.classList.add(FOUND_CLASS);
        setTimeout(() => row.classList.remove(FOUND_CLASS), 1800);
        row.querySelector<HTMLElement>('input, select, textarea, [role="switch"], [role="combobox"]')?.focus({
            preventScroll: true,
        });
    };
    setTimeout(tick, 0);
}
