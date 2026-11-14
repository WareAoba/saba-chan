import clsx from 'clsx';
import type { ReactNode } from 'react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useStickyClip } from '../../hooks/useStickyClip';
import type { Server } from '../../stores/useServerStore';
import type { ModuleInfo } from '../../types/daemon';
import { screenFallbackNotice } from '../../utils/openScreen';
import { Icon } from '../Icon';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { LinkedText } from '../ui/ScreenLink';
import { Segmented } from '../ui/StatusChip';
import { Avatar, initialOf } from './Avatar';
import { BotSettingsTab, botSettingsAvailable } from './BotSettingsTab';
import { ChatbotGuildTab } from './ChatbotGuildTab';
import { chatbotAvailable } from './ChatbotView';
import { ControllersTab } from '../servers/ControllersTab';
import { AccessMode, accessOf, KickButton, MembersKick } from './InstanceAccess';
import * as P from './nodePermissions';
import { ModerationTab, moderationAvailable } from './ModerationTab';
import { MembersPanel, PermissionPanel } from './PermissionsTab';
import { SleepCorner } from './SleepCorner';
import type { DiscordMember, DiscordNode } from './types';

export { Avatar, initialOf };

interface ChoiceCardsProps {
    name: string;
    value: string;
    onChange: (value: string) => void;
    options: { value: string; title: ReactNode; desc: ReactNode }[];
}

/** 둘 중 하나 고르는 카드 (진짜 라디오 버튼이라 키보드로도 고를 수 있다) */
function ChoiceCards({ name, value, onChange, options }: ChoiceCardsProps) {
    return (
        <div className="dc-choice">
            {options.map((o) => (
                <label key={String(o.value)} className={clsx('dc-choice-item', { active: value === o.value })}>
                    <input
                        type="radio"
                        name={name}
                        className="dc-radio-input"
                        checked={value === o.value}
                        onChange={() => onChange(o.value)}
                    />
                    <span className="dc-radio" />
                    <span className="dc-choice-text">
                        <strong>{o.title}</strong>
                        <small>{o.desc}</small>
                    </span>
                </label>
            ))}
        </div>
    );
}

/** 인스턴스의 모듈 아이콘 (서버 목록과 같다). 아이콘이 없는 모듈은 이름 첫 글자 */
function ModuleIcon({ module, name }: { module?: ModuleInfo; name: string }) {
    return (
        <span className={clsx('dc-srv-icon', { 'has-img': !!module?.icon })}>
            {module?.icon ? <img src={module.icon} alt="" /> : initialOf(name)}
        </span>
    );
}

/** 노드에서 보이는 인스턴스 (제한 없음이면 전부) */
function visibleServers(settings: P.NodeSettings, guildId: string, servers: Server[]) {
    return servers.filter((s) => P.isInstanceAllowed(settings, guildId, s.id));
}

/** 켜짐·꺼짐 알림 / 자리 꽉 참 알림 저장 */
type NoticeUpdater = (guildId: string, serverId: string, on: boolean) => unknown;

interface InstancesTabProps {
    node: DiscordNode;
    nodes: DiscordNode[];
    servers: Server[];
    modules?: ModuleInfo[];
    settings: P.NodeSettings;
    update: (fn: (s: P.NodeSettings) => P.NodeSettings) => unknown;
    updateStartNotice: NoticeUpdater;
    updateFullNotice: NoticeUpdater;
    onOpen: (instance: string) => void;
}

// ── 인스턴스 탭 ──
function InstancesTab({
    node,
    nodes,
    servers,
    modules = [],
    settings,
    update,
    updateStartNotice,
    updateFullNotice,
    onOpen,
}: InstancesTabProps) {
    const { t } = useTranslation('gui');
    const mode = P.instanceMode(settings, node.guildId);
    const nodeIds = nodes.map((n) => n.guildId);
    const nameOf = (id: string) => nodes.find((n) => n.guildId === id)?.guildName || id;
    const noticed = P.nodeConfig(settings, node.guildId).startNotice;
    const fullNoticed = P.nodeConfig(settings, node.guildId).fullNotice;

    return (
        <div className="dc-tab">
            <ChoiceCards
                name={`mode-${node.guildId}`}
                value={mode}
                onChange={(m) =>
                    update((s) =>
                        P.setInstanceMode(
                            s,
                            node.guildId,
                            m as 'all' | 'pick',
                            servers.map((x) => x.id),
                            nodeIds,
                        ),
                    )
                }
                options={['all', 'pick'].map((m) => ({
                    value: m,
                    title: t(`discord.instances_${m}`),
                    desc: t(`discord.instances_${m}_desc`),
                }))}
            />

            {servers.length > 0 && (
                <p className="dc-hint">
                    <LinkedText i18nKey="discord.start_notice_hint" to={`discord/${node.guildId}/moderation/channels`} />
                </p>
            )}
            {servers.length > 0 && <p className="dc-hint">{t('discord.full_notice_hint')}</p>}
            <div className="dc-toolbar">
                <span className="muted">
                    <LinkedText i18nKey="discord.instance_menu_hint" to={`discord/${node.guildId}/members`} />
                </span>
            </div>
            {servers.length === 0 ? (
                <p className="muted pad">{t('discord.no_instances')}</p>
            ) : (
                <ul className="dc-list">
                    {servers.map((srv) => {
                        const owner = P.instanceOwner(settings, nodeIds, srv.id, node.guildId);
                        const allowed = P.isInstanceAllowed(settings, node.guildId, srv.id);
                        const locked = mode === 'all' || !!owner;
                        const notice = Array.isArray(noticed) && noticed.includes(srv.id);
                        const full = Array.isArray(fullNoticed) && fullNoticed.includes(srv.id);
                        const openable = allowed && !owner;
                        return (
                            // biome-ignore lint/a11y/useKeyWithClickEvents: 카드 어디를 눌러도 권한 메뉴로 (사용자 지정 2026-11-07). 키보드는 오른쪽 화살표 버튼이 받는다
                            <li
                                key={srv.id}
                                className={clsx('dc-list-row', { dim: !allowed, 'is-openable': openable })}
                                onClick={(e) => {
                                    // 줄 안의 버튼·토글을 누른 것은 그 버튼의 일이다
                                    if (!openable || (e.target as HTMLElement).closest('button, input, label, a')) return;
                                    onOpen(srv.id);
                                }}
                            >
                                <ModuleIcon module={modules.find((m) => m.name === srv.module)} name={srv.name} />
                                <span className="dc-list-text">
                                    <span className="dc-list-name">{srv.name}</span>
                                    <span className="dc-list-sub">
                                        {srv.module}
                                        {owner && (
                                            <>
                                                {' · '}
                                                {t('discord.instance_owned', { node: nameOf(owner) })}
                                            </>
                                        )}
                                    </span>
                                </span>
                                <button data-spot="discord.start_notice"
                                    type="button"
                                    className={clsx('icon-btn dc-notice-btn', { active: notice })}
                                    aria-pressed={notice}
                                    aria-label={t('discord.start_notice', { name: srv.name })}
                                    data-tooltip={t('discord.start_notice', { name: srv.name })}
                                    disabled={!notice && (!allowed || !!owner)}
                                    onClick={() => updateStartNotice(node.guildId, srv.id, !notice)}
                                >
                                    <Icon name="bell" size="sm" />
                                </button>
                                <button
                                    type="button"
                                    className={clsx('icon-btn dc-notice-btn', { active: full })}
                                    aria-pressed={full}
                                    aria-label={t('discord.full_notice', { name: srv.name })}
                                    data-tooltip={t('discord.full_notice', { name: srv.name })}
                                    disabled={!full && (!allowed || !!owner)}
                                    onClick={() => updateFullNotice(node.guildId, srv.id, !full)}
                                >
                                    <Icon name="user" size="sm" />
                                </button>
                                <SabaToggle
                                    checked={allowed && !owner}
                                    disabled={locked}
                                    aria-label={srv.name}
                                    onChange={() => update((s) => P.toggleInstance(s, node.guildId, srv.id, nodeIds))}
                                />
                                <button
                                    type="button"
                                    className="icon-btn"
                                    aria-label={t('discord.instance_menu_open', { name: srv.name })}
                                    data-tooltip={t('discord.instance_menu_open', { name: srv.name })}
                                    disabled={!allowed || !!owner}
                                    onClick={() => onOpen(srv.id)}
                                >
                                    <Icon name="chevronRight" size="sm" />
                                </button>
                            </li>
                        );
                    })}
                </ul>
            )}
        </div>
    );
}

interface InstanceMenuProps {
    node: DiscordNode;
    servers: Server[];
    instance: string;
    onBack: () => void;
}

/**
 * 인스턴스 메뉴: 사용 모드. 서버 공용이면 레벨별 명령 계단(이 인스턴스 + 서버 전체 명령)과 자동 강퇴 명단,
 * 전용 멤버면 전용 멤버 명단과 자동 강퇴 스위치. 멤버 레벨은 서버의 멤버 탭에서 정한다.
 */
function InstanceMenu({ node, servers, instance, onBack }: InstanceMenuProps) {
    const { t } = useTranslation('gui');
    const srv = servers.find((s) => s.id === instance);
    if (!srv) return null;
    return (
        <div className="dc-tab">
            <button type="button" className="dc-crumb" onClick={onBack}>
                <Icon name="chevronLeft" size="sm" /> {t('discord.back_to_instances')}
            </button>
            <h2 className="dc-menu-title">
                {srv.name}
                {srv.module && <small className="muted"> · {srv.module}</small>}
            </h2>
            {/* 사용 모드와 명단은 인스턴스 하나에 붙는다: 이은 모든 디스코드 서버에 같이 적용된다 */}
            {/* 서버 공용의 자동 강퇴는 모드 줄의 작은 단추: 명령 계단의 저장 막대가 늘 맨 아래에 온다 */}
            <AccessMode server={srv}>{accessOf(srv) === 'shared' && <KickButton server={srv} />}</AccessMode>
            {accessOf(srv) === 'members' ? (
                <>
                    <ControllersTab server={srv} />
                    <MembersKick server={srv} />
                </>
            ) : (
                <PermissionPanel node={node} instance={srv.id} />
            )}
        </div>
    );
}

interface NodeViewProps {
    modules?: ModuleInfo[];
    node: DiscordNode;
    nodes: DiscordNode[];
    servers: Server[];
    settings: P.NodeSettings;
    updateAllowed: (guildId: string, fn: (s: P.NodeSettings) => P.NodeSettings) => unknown;
    updateStartNotice: NoticeUpdater;
    updateFullNotice: NoticeUpdater;
    members?: DiscordMember[];
    membersLoading?: boolean;
    membersFailed?: boolean;
    membersPartial?: string | null;
    onRefreshMembers?: () => void;
    saveState?: 'saving' | 'saved' | null;
    sampleModuleWord?: string;
    /** 화면 주소 링크로 연 탭·안 화면 (seq가 바뀌면 다시 연다) */
    request?: { tab?: string; sub?: string; seq: number };
}

/**
 * 디스코드 서버(노드) 하나: 인스턴스(→ 인스턴스 메뉴) / 멤버 / 말하는 사바쨩 / 노래하는 사바쨩 / 읽어주는 사바쨩 / 서버 관리 / 봇 설정.
 */
export function NodeView({
    modules = [],
    node,
    nodes,
    servers,
    settings,
    updateAllowed,
    updateStartNotice,
    updateFullNotice,
    members,
    membersLoading,
    membersFailed,
    membersPartial,
    onRefreshMembers,
    saveState,
    sampleModuleWord,
    request,
}: NodeViewProps) {
    const { t } = useTranslation('gui');
    const [tab, setTab] = useState('instances');
    const [opened, setInstance] = useState<string | null>(null); // 연 인스턴스 메뉴 (없으면 목록)
    const mode = P.instanceMode(settings, node.guildId);
    const visibleCount = visibleServers(settings, node.guildId, servers).length;
    // 보이는 인스턴스만 메뉴로 연다 (지웠거나 이 서버에서 뺀 인스턴스면 목록)
    const instance = opened && visibleServers(settings, node.guildId, servers).some((s) => s.id === opened) ? opened : null;
    const tabs = [
        { id: 'instances', label: t('discord.tab_instances'), icon: <Icon name="server" size="sm" /> },
        { id: 'members', label: t('discord.tab_members'), icon: <Icon name="user" size="sm" /> },
        ...(moderationAvailable()
            ? [{ id: 'moderation', label: t('discord.tab_moderation'), icon: <Icon name="tool" size="sm" /> }]
            : []),
        ...(botSettingsAvailable()
            ? [{ id: 'bot', label: t('discord.tab_bot'), icon: <Icon name="settings" size="sm" /> }]
            : []),
        ...(chatbotAvailable()
            ? [{ id: 'chatbot', label: t('chatbot.nav'), icon: <Icon name="sabaTalking" size="sm" /> }]
            : []),
        ...(botSettingsAvailable()
            ? [
                  {
                      id: 'music',
                      label: t('discord.bot_settings.music_title'),
                      icon: <Icon name="musicNote" size="sm" />,
                  },
                  {
                      id: 'tts',
                      label: t('discord.bot_settings.tts_title'),
                      icon: <Icon name="sabaReading" size="sm" />,
                  },
              ]
            : []),
    ];

    // 화면 주소 링크로 연 탭. 이 디스코드 서버에서 쓸 수 없는 탭이면 지금 탭에 두고 이유를 알린다
    // biome-ignore lint/correctness/useExhaustiveDependencies: 링크를 누를 때(seq)만
    useEffect(() => {
        if (!request?.tab) return;
        if (!tabs.some((x) => x.id === request.tab)) {
            screenFallbackNotice('tab_missing');
            return;
        }
        setTab(request.tab);
        setInstance(null);
    }, [request?.seq]);

    const topRef = useRef<HTMLDivElement>(null);
    useStickyClip(topRef);

    return (
        <div className="dc-view">
            {/* 서버 정보 카드와 탭 바는 위에 고정하고 그 아래만 스크롤한다 (사용자 11-07) */}
            <div className="sticky-head" ref={topRef}>
                <header className="detail-head">
                    <div className="detail-icon">
                        <Avatar name={node.guildName || node.guildId} url={node.iconUrl} size={56} />
                    </div>
                    <div className="detail-title">
                        <h1>{node.guildName || node.guildId}</h1>
                        <div className="detail-sub">
                            <span className="muted">
                                {mode === 'all'
                                    ? t('discord.node_sub_all')
                                    : t('discord.node_sub_pick', { count: visibleCount })}
                            </span>
                        </div>
                    </div>
                    <SleepCorner guildId={node.guildId} />
                    {saveState && (
                        <span className={clsx('dc-save-state', saveState)}>
                            {saveState === 'saving' ? <SabaSpinner size="xs" /> : <Icon name="check" size="xs" color="var(--ok)" />}
                            {saveState === 'saving' ? t('discord.saving') : t('discord.saved')}
                        </span>
                    )}
                </header>

                <Segmented
                    spot="discord.tab"
                    ariaLabel={node.guildName}
                    value={tab}
                    onChange={(v) => {
                        setTab(v);
                        setInstance(null);
                    }}
                    items={tabs}
                />
            </div>

            {membersFailed && !membersLoading && (members?.length ?? 0) > 0 && tab === 'members' && (
                <p className="dc-field-error">
                    <Icon name="info" size="xs" /> {t('discord.members_refresh_failed')}
                </p>
            )}
            {membersPartial && !membersFailed && !membersLoading && tab === 'members' && (
                <p className="dc-field-error">
                    <Icon name="info" size="xs" /> {t('discord.members_partial')}
                </p>
            )}
            {tab === 'instances' && instance && (
                <InstanceMenu
                    key={`${node.guildId}:${instance}`}
                    node={node}
                    servers={visibleServers(settings, node.guildId, servers)}
                    instance={instance}
                    onBack={() => setInstance(null)}
                />
            )}
            {tab === 'members' && (
                <MembersPanel
                    key={node.guildId}
                    node={node}
                    members={members}
                    membersLoading={membersLoading}
                    onRefreshMembers={onRefreshMembers}
                />
            )}
            {tab === 'instances' && !instance && (
                <InstancesTab
                    onOpen={setInstance}
                    node={node}
                    nodes={nodes}
                    servers={servers}
                    modules={modules}
                    settings={settings}
                    update={(fn) => updateAllowed(node.guildId, fn)}
                    updateStartNotice={updateStartNotice}
                    updateFullNotice={updateFullNotice}
                />
            )}
            {tab === 'moderation' && (
                <ModerationTab
                    key={node.guildId}
                    initialView={request?.tab === 'moderation' ? request.sub : undefined}
                    viewSeq={request?.seq}
                    node={node}
                    members={members}
                    membersLoading={membersLoading}
                    onRefreshMembers={onRefreshMembers}
                />
            )}
            {tab === 'chatbot' && <ChatbotGuildTab key={node.guildId} node={node} />}
            {tab === 'music' && <BotSettingsTab key={node.guildId} node={node} section="music" />}
            {tab === 'tts' && <BotSettingsTab key={node.guildId} node={node} section="tts" />}
            {tab === 'bot' && <BotSettingsTab key={node.guildId} node={node} sampleModuleWord={sampleModuleWord} />}
        </div>
    );
}
