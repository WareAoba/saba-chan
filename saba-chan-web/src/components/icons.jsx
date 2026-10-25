import { Icon as GuiIcon } from '@gui/components/Icon';

/** GUI 에 없는 것만 여기 둔다 (24 격자, 글자색, GUI 아이콘과 같은 선 굵기) */
const LOCAL = {
    members: (
        <>
            <path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" />
            <circle cx="9" cy="7" r="4" />
            <path d="M22 21v-2a4 4 0 0 0-3-3.87M16 3.13a4 4 0 0 1 0 7.75" />
        </>
    ),
    more: (
        <>
            <circle cx="5" cy="12" r="1" />
            <circle cx="12" cy="12" r="1" />
            <circle cx="19" cy="12" r="1" />
        </>
    ),
};

/** 웹에서 부르는 이름 → GUI 아이콘 이름 */
const GUI_NAMES = {
    servers: 'server',
    modules: 'extension',
    back: 'chevronLeft',
    close: 'x',
    lock: 'key',
};

export function Icon({ name, size = 20 }) {
    if (!LOCAL[name]) return <GuiIcon name={GUI_NAMES[name] || name} size={size} />;
    return (
        <svg
            width={size}
            height={size}
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
            style={{ display: 'block', flexShrink: 0 }}
        >
            {LOCAL[name]}
        </svg>
    );
}
