import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import { Icon } from '../Icon';
import { SabaSpinner } from '../ui/SabaUI';
import { Segmented } from '../ui/StatusChip';
import type { ChannelAssign } from './ChannelsTab';
import { ChannelsView, StaleAssignments } from './ModerationChannels';
import { MembersView } from './ModerationMembers';
import { modErrorText, type ModState } from './ModerationParts';
import type { DiscordMember, DiscordNode } from './types';
import { failed } from '../../utils/apiResult';


/**
 * 디스코드 서버 화면 › 서버 관리 (2026-10-31): 디스코드 명령 /청소·/자동청소·/슬로우·/잠금·/음소거 와 채널 정보 수정,
 * 사바쨩 채널 배정(예전 "채널" 탭)을 한 화면에서 한다. 앱은 데몬 API 만 부르고, 데몬이 서버쨩에 넘긴다.
 * 채널 상태·음소거: GET/PATCH/POST/PUT /api/relay/node/:guild/moderation… , 배정: GET/PUT /api/relay/node/:guild/channels
 */

export const moderationAvailable = () =>
    typeof window.api?.relayNodeModeration === 'function' &&
    typeof window.api?.relayNodeChannels === 'function' &&
    typeof window.api?.relayNodeChannelsSave === 'function';

interface ModerationTabProps {
    node: DiscordNode;
    members?: DiscordMember[];
    membersLoading?: boolean;
    onRefreshMembers?: () => void;
    /** 화면 주소 링크로 연 안 화면 (channels·members). viewSeq가 바뀌면 다시 연다 */
    initialView?: string;
    viewSeq?: number;
}

/**
 * 서버 관리 탭 — 채널 / 멤버 두 화면.
 * members: 권한 화면과 같은 멤버 목록 (음소거할 사람을 고른다)
 */
export function ModerationTab({
    node,
    members = [],
    membersLoading,
    onRefreshMembers,
    initialView,
    viewSeq,
}: ModerationTabProps) {
    const { t } = useTranslation('gui');
    const guildId = node.guildId;
    const [view, setView] = useState(initialView === 'members' ? 'members' : 'channels');
    // biome-ignore lint/correctness/useExhaustiveDependencies: 링크를 누를 때(viewSeq)만
    useEffect(() => {
        if (initialView === 'channels' || initialView === 'members') setView(initialView);
    }, [viewSeq]);
    const [mod, setMod] = useState<ModState | null>(null);
    const [assign, setAssign] = useState<ApiBody | null>(null);
    // 배정은 서버 전체 값이라 한 번에 하나만 저장한다 (다음 변경은 직전 저장 결과에서 계산)
    const [assignSaving, setAssignSaving] = useState(false);
    const saveAssign = useCallback(
        async (body: ChannelAssign) => {
            setAssignSaving(true);
            try {
                const res = await window.api.relayNodeChannelsSave(guildId, body);
                if (!failed(res)) setAssign(res);
                return res;
            } finally {
                setAssignSaving(false);
            }
        },
        [guildId],
    );
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState<ApiBody | null>(null);

    const load = useCallback(async () => {
        setLoading(true);
        setError(null);
        try {
            const [m, a] = await Promise.all([
                window.api.relayNodeModeration(guildId),
                window.api.relayNodeChannels(guildId),
            ]);
            if (failed(m)) setError(m || {});
            else setMod(m as ModState);
            if (!failed(a)) setAssign(a);
        } catch (e) {
            setError({ message: (e as Error).message });
        }
        setLoading(false);
    }, [guildId]);

    useEffect(() => {
        load();
    }, [load]);

    if (loading && !mod) {
        return (
            <p className="muted pad">
                <SabaSpinner size="sm" /> {t('discord.mod.loading')}
            </p>
        );
    }

    return (
        <div className="dc-tab dc-mod">
            <div className="dc-mod-head">
                <Segmented
                    ariaLabel={t('discord.tab_moderation')}
                    value={view}
                    onChange={setView}
                    items={[
                        { id: 'channels', label: t('discord.mod.view_channels'), icon: <Icon name="hash" size="sm" /> },
                        { id: 'members', label: t('discord.mod.view_members'), icon: <Icon name="user" size="sm" /> },
                    ]}
                />
                <button type="button" className="btn btn-ghost btn-sm" onClick={load} disabled={loading}>
                    <Icon name="refresh" size="sm" /> {t('discord.mod.reload')}
                </button>
            </div>
            {error && (
                <p className="dc-field-error" role="alert">
                    <Icon name="alertTriangle" size="xs" /> {modErrorText(error, t)}
                </p>
            )}
            {mod && !mod.botAvailable && (
                <div className="notice tone-warn" role="status">
                    <Icon name="alertTriangle" size="sm" />
                    <span>{t('discord.mod.no_bot')}</span>
                </div>
            )}
            {view === 'channels' && assign && (
                <StaleAssignments
                    known={mod?.channels || []}
                    assignRes={assign}
                    saveAssign={saveAssign}
                    assignSaving={assignSaving}
                />
            )}
            {mod?.botAvailable && view === 'channels' && (
                <ChannelsView
                    guildId={guildId}
                    mod={mod}
                    setMod={setMod}
                    assignRes={assign}
                    saveAssign={saveAssign}
                    assignSaving={assignSaving}
                    members={members}
                />
            )}
            {mod?.botAvailable && view === 'members' && (
                <MembersView
                    guildId={guildId}
                    mod={mod}
                    reload={load}
                    members={members}
                    membersLoading={membersLoading}
                    onRefreshMembers={onRefreshMembers}
                />
            )}
        </div>
    );
}
