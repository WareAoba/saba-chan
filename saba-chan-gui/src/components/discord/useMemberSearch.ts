import type { PointerEvent as ReactPointerEvent } from 'react';
import { useMemo, useState } from 'react';
import { isSnowflake, MEMBER_PAGE, searchMembers } from './levels';
import type { DiscordMember } from './types';

/**
 * 멤버 찾기 상태 (앱·웹 `MemberSearch`가 같이 쓴다, 그리는 모양은 화면마다 다르다): 닉네임으로 찾고,
 * 결과는 10명씩 보이며 목록 끝까지 내리면 더 보인다. 끄는 결과 칩은 팝업(스크롤 칸) 밖으로 띄워야 잘리지 않으므로
 * 누른 자리를 적어 두고 끄는 동안 fixed로 그 자리에 둔다
 */
export function useMemberSearch(
    members: DiscordMember[],
    nameOf: (id: string) => string,
    onPick: ((id: string) => void) | undefined,
    dragging: string | null | undefined,
) {
    const [query, setQuery] = useState('');
    const [shown, setShown] = useState(MEMBER_PAGE);
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
    /** 끄는 동안 팝업 밖에 띄운 결과 칩의 자리 (띄우지 않으면 없다) */
    const floatStyle = (mid: string) =>
        dragging === mid && origin?.id === mid ? { left: origin.left, top: origin.top, width: origin.width } : undefined;
    return {
        query,
        id,
        results,
        shown: results.slice(0, shown),
        open: !!id,
        // 멤버 목록에 없는 사람은 디스코드 ID를 그대로 적어 더한다
        byId: !!onPick && isSnowflake(id) && !members.some((m) => String(m.id) === id),
        withOrigin,
        floatStyle,
        change: (value: string) => {
            setQuery(value);
            setShown(MEMBER_PAGE);
        },
        clear: () => setQuery(''),
        pick: (mid: string) => {
            if (!onPick) return;
            onPick(mid);
            setQuery('');
        },
        /** 목록 끝까지 내리면 더 보인다 */
        scroll: (el: HTMLElement) => {
            if (el.scrollTop + el.clientHeight >= el.scrollHeight - 24 && shown < results.length)
                setShown((n) => n + MEMBER_PAGE);
        },
    };
}
