import { createContext, useCallback, useContext, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';

const EDGE = 4;
// 단축키 표기: macOS는 ⌘, 그 밖은 Ctrl+
const MOD = typeof navigator !== 'undefined' && /Mac/i.test(navigator.platform || '') ? '⌘' : 'Ctrl+';
const ContextMenuContext = createContext(null);

/** 메뉴 자리: 커서에 두고, 화면 밖으로 나가면 안쪽으로 민다 */
export function computeMenuPosition(x, y, w, h, vpW, vpH) {
    return {
        left: Math.max(EDGE, Math.min(x, vpW - w - EDGE)),
        top: Math.max(EDGE, Math.min(y, vpH - h - EDGE)),
    };
}

/**
 * 대상이 있는 곳의 우클릭 메뉴를 연다: `onContextMenu={(e) => openMenu(e, items)}`.
 * items: `{ id, label, icon?, onSelect, disabled?, danger?, shortcut? }` 또는 `'separator'`. 거짓 값은 뺀다.
 */
export function useContextMenu() {
    return useContext(ContextMenuContext) ?? NOOP;
}
const NOOP = () => {};

const TEXT_INPUT = new Set(['text', 'search', 'url', 'tel', 'email', 'password', 'number', '']);

function editableOf(target) {
    const el = target?.closest?.('input, textarea, [contenteditable="true"], [contenteditable=""]');
    if (!el) return null;
    if (el.tagName === 'INPUT' && !TEXT_INPUT.has((el.getAttribute('type') || '').toLowerCase())) return null;
    return el;
}

/** 입력칸 편집 메뉴. 고른 범위는 메뉴를 열 때 잡아 둔다 (메뉴를 누르면 포커스가 바뀔 수 있다) */
function editItems(t, el) {
    const isField = el.tagName === 'INPUT' || el.tagName === 'TEXTAREA';
    // number·email 칸은 선택 범위를 다룰 수 없다(selectionStart가 null, setSelectionRange는 예외)
    const hasRange = isField && el.selectionStart !== null;
    const start = hasRange ? el.selectionStart : 0;
    const end = hasRange ? el.selectionEnd : 0;
    const selected = hasRange ? el.value.slice(start, end) : isField ? '' : String(window.getSelection() || '');
    const locked = el.readOnly || el.disabled || el.getAttribute('contenteditable') === 'false';
    const secret = el.type === 'password';
    const restore = () => {
        el.focus();
        if (hasRange) el.setSelectionRange(start, end);
    };
    return [
        {
            id: 'undo',
            label: t('context_menu.undo'),
            icon: 'undo',
            shortcut: `${MOD}Z`,
            disabled: locked,
            onSelect: () => {
                el.focus();
                document.execCommand('undo');
            },
        },
        'separator',
        {
            id: 'cut',
            label: t('context_menu.cut'),
            icon: 'scissors',
            shortcut: `${MOD}X`,
            disabled: locked || secret || !selected,
            onSelect: async () => {
                await navigator.clipboard?.writeText(selected);
                restore();
                document.execCommand('delete');
            },
        },
        {
            id: 'copy',
            label: t('context_menu.copy'),
            icon: 'copy',
            shortcut: `${MOD}C`,
            disabled: secret || !selected,
            onSelect: () => navigator.clipboard?.writeText(selected),
        },
        {
            id: 'paste',
            label: t('context_menu.paste'),
            icon: 'clipboard',
            shortcut: `${MOD}V`,
            disabled: locked,
            onSelect: async () => {
                const text = await navigator.clipboard?.readText?.();
                if (!text) return;
                restore();
                // insertText는 입력 이벤트를 내서 React onChange와 실행 취소 기록이 그대로 동작한다
                document.execCommand('insertText', false, text);
            },
        },
        'separator',
        {
            id: 'select_all',
            label: t('context_menu.select_all'),
            icon: 'selectAll',
            shortcut: `${MOD}A`,
            onSelect: () => {
                el.focus();
                if (isField) el.select();
                else document.execCommand('selectAll');
            },
        },
    ];
}

/** 아무 곳에서나 글자를 골라 두고 우클릭했을 때 */
function copyItems(t, text) {
    return [
        {
            id: 'copy',
            label: t('context_menu.copy'),
            icon: 'copy',
            shortcut: `${MOD}C`,
            onSelect: () => navigator.clipboard?.writeText(text),
        },
    ];
}

const usable = (items) => items.filter(Boolean);
const selectable = (item) => item && item !== 'separator' && !item.disabled;

/**
 * 앱 전체의 우클릭을 맡는다. 대상이 메뉴를 열지 않은 곳은 입력칸이면 편집 메뉴, 고른 글자가 있으면 복사,
 * 그 밖은 WebView 기본 메뉴(뒤로·새로고침·검사)를 막는다. 개발 빌드는 디버깅용으로 기본 메뉴를 둔다.
 */
export function ContextMenuProvider({ children, allowNative = import.meta.env.DEV }) {
    const { t } = useTranslation('gui');
    const [menu, setMenu] = useState(null); // { x, y, items, pos? }
    const [active, setActive] = useState(-1);
    const menuRef = useRef(null);

    const close = useCallback(() => setMenu(null), []);

    const open = useCallback(
        (e, items) => {
            e.preventDefault();
            e.stopPropagation?.();
            // 대상 안의 입력칸(플러그인 설정 칸 등)은 대상 메뉴보다 편집 메뉴가 먼저다
            const editable = editableOf(e.target);
            const list = usable(editable ? editItems(t, editable) : items);
            if (!list.some((i) => i !== 'separator')) return;
            setActive(-1);
            setMenu({ x: e.clientX, y: e.clientY, items: list });
        },
        [t],
    );

    useEffect(() => {
        // React의 onContextMenu는 루트에서 먼저 돈다: 대상이 메뉴를 열었으면 여기서는 defaultPrevented
        const onContextMenu = (e) => {
            if (e.defaultPrevented) return;
            const editable = editableOf(e.target);
            if (editable) return open(e, editItems(t, editable));
            const selected = String(window.getSelection() || '').trim();
            if (selected) return open(e, copyItems(t, selected));
            setMenu(null);
            if (!allowNative) e.preventDefault();
        };
        document.addEventListener('contextmenu', onContextMenu);
        return () => document.removeEventListener('contextmenu', onContextMenu);
    }, [open, t, allowNative]);

    const run = useCallback(
        (item) => {
            if (!selectable(item)) return;
            close();
            try {
                item.onSelect?.()?.catch?.(() => {});
            } catch {
                // 메뉴 동작의 실패는 각 동작이 알린다
            }
        },
        [close],
    );

    useEffect(() => {
        if (!menu) return undefined;
        const outside = (e) => {
            if (!menuRef.current?.contains(e.target)) close();
        };
        const onKey = (e) => {
            // 메뉴가 쓴 키는 포커스를 쥔 입력칸까지 보내지 않는다 (콘솔 기록 넘기기, Enter 제출 등)
            const consume = () => {
                e.preventDefault();
                e.stopPropagation();
            };
            if (e.key === 'Escape') {
                consume();
                return close();
            }
            const idx = menu.items.map((item, i) => (selectable(item) ? i : -1)).filter((i) => i >= 0);
            if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                consume();
                const at = idx.indexOf(active);
                const next = e.key === 'ArrowDown' ? (at + 1) % idx.length : (at - 1 + idx.length) % idx.length;
                setActive(idx[at < 0 && e.key === 'ArrowUp' ? idx.length - 1 : next]);
            } else if (e.key === 'Enter' && active >= 0) {
                consume();
                run(menu.items[active]);
            }
        };
        document.addEventListener('pointerdown', outside, true);
        document.addEventListener('keydown', onKey, true);
        window.addEventListener('scroll', close, true);
        window.addEventListener('resize', close);
        window.addEventListener('blur', close);
        return () => {
            document.removeEventListener('pointerdown', outside, true);
            document.removeEventListener('keydown', onKey, true);
            window.removeEventListener('scroll', close, true);
            window.removeEventListener('resize', close);
            window.removeEventListener('blur', close);
        };
    }, [menu, active, close, run]);

    // 처음엔 안 보이게 그려 크기를 잰 뒤 자리를 잡는다
    useLayoutEffect(() => {
        if (!menu || menu.pos || !menuRef.current) return;
        const el = menuRef.current;
        const pos = computeMenuPosition(
            menu.x,
            menu.y,
            el.offsetWidth,
            el.offsetHeight,
            window.innerWidth,
            window.innerHeight,
        );
        setMenu((prev) => (prev ? { ...prev, pos } : prev));
    }, [menu]);

    return (
        <ContextMenuContext.Provider value={open}>
            {children}
            {menu &&
                createPortal(
                    <div
                        ref={menuRef}
                        role="menu"
                        className="context-menu"
                        style={menu.pos ? menu.pos : { top: -9999, left: -9999, visibility: 'hidden' }}
                        onContextMenu={(e) => e.preventDefault()}
                    >
                        {menu.items.map((item, i) =>
                            item === 'separator' ? (
                                // biome-ignore lint/suspicious/noArrayIndexKey: 구분선은 자리로만 구분된다
                                <hr key={`sep-${i}`} className="context-menu-separator" />
                            ) : (
                                <button
                                    key={item.id}
                                    type="button"
                                    role="menuitem"
                                    className={`context-menu-item${item.danger ? ' danger' : ''}${i === active ? ' active' : ''}`}
                                    disabled={item.disabled}
                                    // 입력칸의 포커스와 고른 범위를 빼앗지 않는다
                                    onMouseDown={(e) => e.preventDefault()}
                                    onMouseEnter={() => setActive(selectable(item) ? i : -1)}
                                    onClick={() => run(item)}
                                >
                                    {item.icon && <Icon name={item.icon} size="sm" />}
                                    <span className="context-menu-label">{item.label}</span>
                                    {item.shortcut && (
                                        <span className="context-menu-shortcut" aria-hidden="true">
                                            {item.shortcut}
                                        </span>
                                    )}
                                </button>
                            ),
                        )}
                    </div>,
                    document.body,
                )}
        </ContextMenuContext.Provider>
    );
}
