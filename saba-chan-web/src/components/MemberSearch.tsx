import * as L from '@gui/components/discord/levels';
import type { DiscordMember } from '@gui/components/discord/types';
import type { PointerEvent as ReactPointerEvent } from 'react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from './icons';
import { Avatar } from './ui';

/** 검색 결과를 한 번에 보이는 수. 목록 끝까지 내리면 이만큼 더 보인다 (사용자 지정 2026-11-07, 앱과 같다) */
export const MEMBER_PAGE = 10;

export const isSnowflake = (v: string) => /^\d{17,20}$/.test(v); // 디스코드 ID

/** 서버 닉네임(표시 이름)·전역 이름·사용자 이름으로 찾는다. 아이디로는 찾지 않는다 */
export function searchMembers(members: DiscordMember[], query: string, name: (m: DiscordMember) => string) {
    const q = query.trim().toLowerCase();
    if (!q) return [];
    const hit = members.filter((m) =>
        [m.displayName, m.globalName, m.username].some((v) => typeof v === 'string' && v.toLowerCase().includes(q)),
    );
    return L.sortByName(hit, name);
}

interface MemberSearchProps {
    members: DiscordMember[];
    label: string;
    nameOf: (id: string) => string;
    avatarOf: (id: string) => unknown;
    /** 고르면 (사람 추가). 없으면 결과는 끌기만 한다 */
    onPick?: (id: string) => void;
    /** 결과 칩에 펼칠 끌기 속성 (위쪽 검색바, @gui pointerDrag `bind`) */
    dragProps?: (id: string) => Record<string, unknown>;
    dragging?: string | null;
    disabled?: boolean;
    autoFocus?: boolean;
}

/**
 * 멤버 찾기 (앱 `MemberSearch`와 같다): 닉네임으로 찾고, 결과는 10명씩 보이며 목록 끝까지 내리면 더 보인다.
 * 사람 추가에서는 결과를 눌러 고르고, 위쪽 검색바에서는 결과를 끌어 레벨 칸에 놓는다
 */
export function MemberSearch({
    members,
    label,
    nameOf,
    avatarOf,
    onPick,
    dragProps,
    dragging,
    disabled,
    autoFocus,
}: MemberSearchProps) {
    const { t: tg } = useTranslation('gui');
    const [query, setQuery] = useState('');
    const [shown, setShown] = useState(MEMBER_PAGE);
    // 끄는 결과 칩은 팝업(스크롤 칸) 밖으로 띄워야 잘리지 않는다: 누른 자리를 적어 두고 끄는 동안 fixed로 그 자리에 둔다
    const [origin, setOrigin] = useState<{ id: string; left: number; top: number; width: number } | null>(null);
    const results = useMemo(() => searchMembers(members, query, (m) => nameOf(String(m.id))), [members, query, nameOf]);
    const withOrigin = (mid: string, props: Record<string, unknown>) => ({
        ...props,
        onPointerDown: (e: ReactPointerEvent<HTMLElement>) => {
            const box = e.currentTarget.getBoundingClientRect();
            setOrigin({ id: mid, left: box.left, top: box.top, width: box.width });
            (props.onPointerDown as ((ev: ReactPointerEvent<HTMLElement>) => void) | undefined)?.(e);
        },
    });
    const id = query.trim();
    // 멤버 목록에 없는 사람은 디스코드 ID를 그대로 적어 더한다
    const byId = onPick && isSnowflake(id) && !members.some((m) => String(m.id) === id);
    const open = !!id;
    return (
        <div className={`member-search${dragging ? ' is-dragging' : ''}`}>
            <input
                className="search"
                role="combobox"
                aria-expanded={open}
                aria-label={label}
                placeholder={tg('discord.perm_member_search')}
                value={query}
                disabled={disabled}
                // biome-ignore lint/a11y/noAutofocus: 사람 추가를 누르면 바로 찾게 한다
                autoFocus={autoFocus}
                onChange={(e) => {
                    setQuery(e.target.value);
                    setShown(MEMBER_PAGE);
                }}
                onKeyDown={(e) => {
                    if (e.key === 'Escape') setQuery('');
                }}
            />
            {query && (
                <button
                    type="button"
                    className="member-clear"
                    aria-label={tg('discord.perm_member_search_clear')}
                    title={tg('discord.perm_member_search_clear')}
                    onClick={() => setQuery('')}
                >
                    <Icon name="clear" size={12} />
                </button>
            )}
            {open && (
                <div
                    className="member-results"
                    role="listbox"
                    aria-label={label}
                    onScroll={(e) => {
                        const el = e.currentTarget;
                        if (el.scrollTop + el.clientHeight >= el.scrollHeight - 24 && shown < results.length)
                            setShown((n) => n + MEMBER_PAGE);
                    }}
                >
                    {results.slice(0, shown).map((m) => {
                        const mid = String(m.id);
                        const floating = dragging === mid && origin?.id === mid;
                        return (
                            <div key={mid} className="member-result" role="none">
                                <button
                                    type="button"
                                    role="option"
                                    aria-selected={false}
                                    aria-label={nameOf(mid)}
                                    className={`person-chip${dragging === mid ? ' is-dragging' : ''}${floating ? ' is-floating' : ''}`}
                                    style={
                                        floating
                                            ? { left: origin.left, top: origin.top, width: origin.width }
                                            : undefined
                                    }
                                    disabled={disabled}
                                    {...(dragProps ? withOrigin(mid, dragProps(mid)) : {})}
                                    onClick={() => {
                                        if (!onPick) return;
                                        onPick(mid);
                                        setQuery('');
                                    }}
                                >
                                    <Avatar url={avatarOf(mid)} name={nameOf(mid)} className="avatar avatar-chip" />
                                    <span>{nameOf(mid)}</span>
                                    {m.username && m.username !== nameOf(mid) && (
                                        <small className="hint">@{m.username}</small>
                                    )}
                                </button>
                            </div>
                        );
                    })}
                    {byId && (
                        <div className="member-result" role="none">
                            <button
                                type="button"
                                role="option"
                                aria-selected={false}
                                className="person-chip is-add"
                                disabled={disabled}
                                onClick={() => {
                                    onPick?.(id);
                                    setQuery('');
                                }}
                            >
                                <Icon name="plus" size={14} /> {tg('discord.perm_add_by_id', { id })}
                            </button>
                        </div>
                    )}
                    {results.length === 0 && !byId && (
                        <div className="hint member-empty" role="none">
                            {tg('discord.perm_member_search_empty')}
                        </div>
                    )}
                </div>
            )}
        </div>
    );
}
