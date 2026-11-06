/** 조작 가능 명단 다루기 (데몬 `controllers::parse`와 같은 규칙. 최종 검사는 데몬이 한다) */

import type { Controller } from '../../types/daemon';
import type { DiscordMember, MembersByGuild } from '../discord/types';

/** 명단에 넣을 수 있는 사람 수와 닉네임 길이 (데몬 `MAX_CONTROLLERS`·`NAME_MAX`) */
export const MAX_CONTROLLERS = 200;
const NAME_MAX = 64;

export type AddError = 'empty' | 'duplicate' | 'too_long' | 'too_many';

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** 한 줄 넣기. 디스코드 멤버는 고르지 않아도 된다 */
export function addController(
    list: Controller[],
    rawName: string,
    discordId: string,
): { list: Controller[] } | { error: AddError } {
    const name = rawName.trim();
    if (!name) return { error: 'empty' };
    if ([...name].length > NAME_MAX) return { error: 'too_long' };
    if (list.some((c) => same(c.name, name))) return { error: 'duplicate' };
    if (list.length >= MAX_CONTROLLERS) return { error: 'too_many' };
    return { list: [...list, discordId ? { name, discord_id: discordId } : { name }] };
}

export const removeController = (list: Controller[], index: number) => list.filter((_, i) => i !== index);

export interface MemberChoice {
    id: string;
    label: string;
    avatarUrl?: string;
}

const memberName = (m: DiscordMember) => m.displayName || m.globalName || m.username || String(m.id);

/** 이 PC에 이은 디스코드 서버들의 멤버를 한 번씩만 (디스코드 화면이 받아 둔 목록) */
export function memberChoices(byGuild: MembersByGuild | null | undefined): MemberChoice[] {
    const seen = new Map<string, MemberChoice>();
    for (const members of Object.values(byGuild || {})) {
        for (const m of members || []) {
            const id = String(m.id);
            if (!id || seen.has(id)) continue;
            seen.set(id, {
                id,
                label: memberName(m),
                avatarUrl: typeof m.avatarUrl === 'string' ? m.avatarUrl : undefined,
            });
        }
    }
    return [...seen.values()].sort((a, b) => a.label.localeCompare(b.label));
}
