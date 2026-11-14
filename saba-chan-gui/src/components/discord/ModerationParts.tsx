import type { TFunction } from 'i18next';
import type { ReactNode } from 'react';
import { useState } from 'react';
import type { ApiBody } from '../../types/daemon';
import { failed } from '../../utils/apiResult';
import { Icon } from '../Icon';
import { type ChannelAssign, channelsPatch, LIST_KINDS } from './ChannelsTab';
import type { DiscordMember } from './types';

/* 서버 관리 탭의 채널 화면(ModerationChannels)과 멤버 화면(ModerationMembers)이 함께 쓰는 조각 */

/** GET /api/relay/node/:guild/moderation 의 채널 하나 */
export interface ModChannel {
    id: string;
    name: string;
    category?: string;
    topic?: string;
    slowmode?: number;
    locked?: { until?: string | number } | null;
    autoClear?: number;
}

/** 음소거 중인 멤버 */
export interface ModMute {
    userId: string;
    name: string;
    avatarUrl?: string;
    forever?: boolean;
    until: string;
}

/** GET /api/relay/node/:guild/moderation */
export interface ModState {
    botAvailable?: boolean;
    channels?: ModChannel[];
    can?: Record<string, boolean>;
    mutes?: ModMute[];
}

/** 채널 하나가 맡는 일 — 한 채널은 한 가지만 (릴레이 setAssignments) */
export const ASSIGN_KINDS = ['chatbot', 'music', 'notify', 'gameChat', 'board'];

export const isList = (k: string) => LIST_KINDS.includes(k);
const listOf = (v: unknown): string[] => (Array.isArray(v) ? v.map(String) : []);

export const assignOf = (res: ApiBody | null | undefined): ChannelAssign => ({
    chatbot: listOf(res?.chatbot),
    music: res?.music ? String(res.music) : null,
    notify: res?.notify ? String(res.notify) : null,
    gameChat: listOf(res?.gameChat),
    board: listOf(res?.board),
});

/** 그 일을 맡은 채널들 */
export const idsOf = (assign: ChannelAssign, k: string) =>
    (isList(k) ? assign[k] || [] : assign[k] ? [assign[k]] : []) as string[];

export function kindOf(assign: ChannelAssign, id: string) {
    return ASSIGN_KINDS.find((k) => idsOf(assign, k).includes(id)) || null;
}

/** 이 채널의 일을 kind 로 (null 이면 뺀다). 여러 채널 일은 더하고, 다른 채널이 맡던 한 채널짜리 일은 이 채널로 옮긴다. 바뀐 키만 */
export function assignPatch(saved: ChannelAssign, id: string, kind: string | null) {
    const draft: ChannelAssign = { ...saved };
    for (const k of ASSIGN_KINDS) {
        if (isList(k)) draft[k] = idsOf(saved, k).filter((x) => x !== id);
        else if (draft[k] === id) draft[k] = null;
    }
    if (kind && isList(kind)) draft[kind] = [...(draft[kind] as string[]), id];
    else if (kind) draft[kind] = id;
    return channelsPatch(saved, draft);
}

/** 데몬·릴레이 오류 본문 → 한 줄 */
export function modErrorText(res: ApiBody | null | undefined, t: TFunction) {
    const code = res?.error_code || res?.code || res?.error;
    if (code === 'BOT_MISSING_PERMISSION') {
        return t('discord.mod.err_BOT_MISSING_PERMISSION', {
            permission: t(`discord.mod.perm_${res?.permission}`, { defaultValue: res?.permission }),
        });
    }
    if (code === 'RENAME_LIMIT') {
        return t('discord.mod.err_RENAME_LIMIT', { minutes: Math.max(1, Math.ceil((res?.retryAfter || 0) / 60)) });
    }
    const known = [
        'NOT_LOCKED',
        'FORBIDDEN',
        'UNKNOWN_CHANNEL',
        'BOT_UNAVAILABLE',
        'NODE_MOVED',
        'MUTE_NOT_MEMBER',
        'MUTE_CANNOT',
        'MUTE_BOT',
        'MUTE_SELF',
        'MUTE_TOO_LONG',
    ];
    return known.includes(code) ? t(`discord.mod.err_${code}`) : t('discord.mod.err_generic');
}

export type ActionResult = { tone: 'ok'; text: string } | { tone: 'error'; res: ApiBody };

/** 한 칸에서 부르는 일 하나 — 진행·오류·결과 한 줄을 그 칸 안에 보인다 */
export function useAction() {
    const [busy, setBusy] = useState(false);
    const [note, setNote] = useState<ActionResult | null>(null); // {tone, text}
    const run = async (call: () => Promise<ApiBody>, onOk?: (res: ApiBody) => ActionResult | null) => {
        setBusy(true);
        setNote(null);
        try {
            const res = await call();
            if (failed(res)) setNote({ tone: 'error', res });
            else setNote(onOk ? onOk(res) : null);
        } catch (e) {
            setNote({ tone: 'error', res: { message: (e as Error).message } });
        }
        setBusy(false);
    };
    return { busy, note, setNote, run };
}

export function ActionNote({ note, errorText }: { note: ActionResult | null; errorText: (res: ApiBody) => string }) {
    if (!note) return null;
    if (note.tone === 'error') {
        return (
            <p className="dc-field-error" role="alert">
                <Icon name="alertTriangle" size="xs" /> {errorText(note.res)}
            </p>
        );
    }
    return (
        <p className="dc-hint dc-mod-ok" role="status">
            <Icon name="checkCircle" size="xs" /> {note.text}
        </p>
    );
}

interface SectionProps {
    icon: string;
    title: ReactNode;
    desc?: ReactNode;
    children?: ReactNode;
    aside?: ReactNode;
}

export function Section({ icon, title, desc, children, aside }: SectionProps) {
    return (
        <section className="dc-card dc-mod-section">
            <div className="dc-card-head">
                <h2>
                    <Icon name={icon} size="sm" /> {title}
                </h2>
                {aside}
            </div>
            {desc && <p className="dc-card-desc">{desc}</p>}
            {children}
        </section>
    );
}

export const memberName = (m: DiscordMember) => m?.displayName || m?.globalName || m?.username || m?.id;
