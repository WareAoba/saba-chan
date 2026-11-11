import clsx from 'clsx';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { SlotView, useSlotPages } from '../ExtensionSlot';
import { Icon } from '../Icon';
import { useContextMenu } from '../ui/ContextMenu';

/** 맨 위 탭 줄에 바로 보일 익스텐션 화면 수. 넘치면 "더보기" 메뉴로 */
export const APP_PAGE_INLINE = 2;

/** 맨 위 탭 줄의 익스텐션 화면 (`App.page`) */
export function ExtensionPageTabs({ page, onNavigate }: { page: string; onNavigate: (page: string) => void }) {
    const { t } = useTranslation('gui');
    const openMenu = useContextMenu();
    const pages = useSlotPages('App.page');
    if (pages.length === 0) return null;
    const inline = pages.slice(0, APP_PAGE_INLINE);
    const more = pages.slice(APP_PAGE_INLINE);
    // 넘친 화면을 보고 있으면 "더보기" 자리에 그 화면을 보인다
    const shownMore = more.find((p) => p.id === page);
    return (
        <>
            {inline.map((p) => (
                <button
                    key={p.id}
                    type="button"
                    className={clsx('appbar-tab', { active: page === p.id })}
                    aria-current={page === p.id ? 'page' : undefined}
                    onClick={() => onNavigate(p.id)}
                >
                    <Icon name={p.icon} size="sm" />
                    <span>{p.label}</span>
                </button>
            ))}
            {more.length > 0 && (
                <button
                    type="button"
                    className={clsx('appbar-tab', { active: !!shownMore })}
                    aria-current={shownMore ? 'page' : undefined}
                    aria-haspopup="menu"
                    onClick={(e) =>
                        openMenu(
                            e,
                            more.map((p) => ({ id: p.id, label: p.label, icon: p.icon, onSelect: () => onNavigate(p.id) })),
                        )
                    }
                >
                    <Icon name={shownMore ? shownMore.icon : 'chevronDown'} size="sm" />
                    <span>{shownMore ? shownMore.label : t('shell.nav_more')}</span>
                </button>
            )}
        </>
    );
}

/** 오른쪽 위 아이콘 버튼 + 누르면 뜨는 작은 창 (`AppBar.actions`) */
export function ExtensionAppBarActions() {
    const pages = useSlotPages('AppBar.actions');
    const [open, setOpen] = useState<string | null>(null);
    const anchorRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
        if (!open) return;
        const onDown = (e: MouseEvent) => {
            if (!anchorRef.current?.contains(e.target as Node)) setOpen(null);
        };
        const onKey = (e: KeyboardEvent) => {
            if (e.key === 'Escape') setOpen(null);
        };
        document.addEventListener('mousedown', onDown);
        document.addEventListener('keydown', onKey);
        return () => {
            document.removeEventListener('mousedown', onDown);
            document.removeEventListener('keydown', onKey);
        };
    }, [open]);

    if (pages.length === 0) return null;
    return (
        <div className="appbar-ext-actions" ref={anchorRef}>
            {pages.map((p) => (
                <div key={p.id} className="popover-anchor">
                    <button
                        type="button"
                        className="icon-btn"
                        data-tooltip={p.label}
                        aria-label={p.label}
                        aria-expanded={open === p.id}
                        onClick={() => setOpen(open === p.id ? null : p.id)}
                    >
                        <Icon name={p.icon} size="sm" />
                    </button>
                    {open === p.id && (
                        <div className="ext-popover" role="dialog" aria-label={p.label}>
                            <SlotView entry={p.entry} slotId="AppBar.actions" close={() => setOpen(null)} />
                        </div>
                    )}
                </div>
            ))}
        </div>
    );
}
