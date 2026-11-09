import type { DiscordMember } from '@gui/components/discord/types';
import { useMemberSearch } from '@gui/components/discord/useMemberSearch';
import { useTranslation } from 'react-i18next';
import { Icon } from './icons';
import { Avatar } from './ui';

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
    const search = useMemberSearch(members, nameOf, onPick, dragging);
    const { query, id, results, open } = search;
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
                onChange={(e) => search.change(e.target.value)}
                onKeyDown={(e) => {
                    if (e.key === 'Escape') search.clear();
                }}
            />
            {query && (
                <button
                    type="button"
                    className="member-clear"
                    aria-label={tg('discord.perm_member_search_clear')}
                    title={tg('discord.perm_member_search_clear')}
                    onClick={search.clear}
                >
                    <Icon name="clear" size={12} />
                </button>
            )}
            {open && (
                <div
                    className="member-results"
                    role="listbox"
                    aria-label={label}
                    onScroll={(e) => search.scroll(e.currentTarget)}
                >
                    {search.shown.map((m) => {
                        const mid = String(m.id);
                        const floating = search.floatStyle(mid);
                        return (
                            <div key={mid} className="member-result" role="none">
                                <button
                                    type="button"
                                    role="option"
                                    aria-selected={false}
                                    aria-label={nameOf(mid)}
                                    className={`person-chip${dragging === mid ? ' is-dragging' : ''}${floating ? ' is-floating' : ''}`}
                                    style={floating}
                                    disabled={disabled}
                                    {...(dragProps ? search.withOrigin(mid, dragProps(mid)) : {})}
                                    onClick={() => search.pick(mid)}
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
                    {search.byId && (
                        <div className="member-result" role="none">
                            <button
                                type="button"
                                role="option"
                                aria-selected={false}
                                className="person-chip is-add"
                                disabled={disabled}
                                onClick={() => search.pick(id)}
                            >
                                <Icon name="plus" size={14} /> {tg('discord.perm_add_by_id', { id })}
                            </button>
                        </div>
                    )}
                    {results.length === 0 && !search.byId && (
                        <div className="hint member-empty" role="none">
                            {tg('discord.perm_member_search_empty')}
                        </div>
                    )}
                </div>
            )}
        </div>
    );
}
