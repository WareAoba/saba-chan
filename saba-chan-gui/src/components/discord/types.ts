/** 디스코드 화면이 함께 쓰는 릴레이 응답 모양 */

/** 릴레이가 돌려주는 길드 하나 (cloudNodes 항목) */
export interface DiscordNode {
    guildId: string;
    guildName?: string;
    iconUrl?: string;
    prefixDefault?: boolean;
    prefixes?: string[];
    [key: string]: unknown;
}

/** 디스코드 멤버 하나 (릴레이 멤버 목록·legacy-members 항목) */
export interface DiscordMember {
    id: string | number;
    displayName?: string;
    globalName?: string;
    username?: string;
    avatarUrl?: string;
    avatar?: string;
    level?: number;
    [key: string]: unknown;
}

/** guildId → 그 서버 멤버 목록 */
export type MembersByGuild = Record<string, DiscordMember[]>;
