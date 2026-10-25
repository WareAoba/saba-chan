import clsx from 'clsx';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { Segmented } from '../ui/StatusChip';
import { BotSettingsTab, botSettingsAvailable } from './BotSettingsTab';
import { ChannelsTab, channelsAvailable } from './ChannelsTab';
import * as P from './nodePermissions';
import { PermissionPanel, permissionsAvailable } from './PermissionsTab';

export const initialOf = (name) => (name || '?').trim().charAt(0).toUpperCase() || '?';

export function Avatar({ name, url, size = 36 }) {
    return (
        <span className="dc-avatar" style={{ width: size, height: size }}>
            {typeof url === 'string' && url.startsWith('http') ? <img src={url} alt="" /> : initialOf(name)}
        </span>
    );
}

/** 둘 중 하나 고르는 카드 (진짜 라디오 버튼이라 키보드로도 고를 수 있다) */
function ChoiceCards({ name, value, onChange, options }) {
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

const memberName = (m) => m.displayName || m.globalName || m.username || m.id;

/** 노드에서 보이는 인스턴스 (제한 없음이면 전부) */
function visibleServers(settings, guildId, servers) {
    return servers.filter((s) => P.isInstanceAllowed(settings, guildId, s.id));
}

// ── 인스턴스 탭 ──
function InstancesTab({ node, nodes, servers, settings, update }) {
    const { t } = useTranslation('gui');
    const mode = P.instanceMode(settings, node.guildId);
    const nodeIds = nodes.map((n) => n.guildId);
    const nameOf = (id) => nodes.find((n) => n.guildId === id)?.guildName || id;

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
                            m,
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

            {servers.length === 0 ? (
                <p className="muted pad">{t('discord.no_instances')}</p>
            ) : (
                <ul className="dc-list">
                    {servers.map((srv) => {
                        const owner = P.instanceOwner(settings, nodeIds, srv.id, node.guildId);
                        const allowed = P.isInstanceAllowed(settings, node.guildId, srv.id);
                        const locked = mode === 'all' || !!owner;
                        return (
                            <li key={srv.id} className={clsx('dc-list-row', { dim: !allowed })}>
                                <span className="dc-srv-icon">{initialOf(srv.name)}</span>
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
                                <SabaToggle
                                    checked={allowed && !owner}
                                    disabled={locked}
                                    aria-label={srv.name}
                                    onChange={() => update((s) => P.toggleInstance(s, node.guildId, srv.id, nodeIds))}
                                />
                            </li>
                        );
                    })}
                </ul>
            )}
        </div>
    );
}

// ── 멤버 탭 ──
function MembersTab({ node, members, loading, settings, onRefresh, onOpenMember }) {
    const { t } = useTranslation('gui');
    const [query, setQuery] = useState('');
    const q = query.trim().toLowerCase();
    const list = (members || []).filter(
        (m) => !q || `${memberName(m)} ${m.username || ''} ${m.id}`.toLowerCase().includes(q),
    );

    return (
        <div className="dc-tab">
            <div className="dc-toolbar">
                <input
                    type="search"
                    className="input"
                    value={query}
                    onChange={(e) => setQuery(e.target.value)}
                    placeholder={t('discord.members_search')}
                    aria-label={t('discord.members_search')}
                />
                <button type="button" className="btn btn-secondary" onClick={onRefresh} disabled={loading}>
                    <Icon name="refresh" size="sm" /> {t('discord.reload')}
                </button>
            </div>
            <p className="dc-hint">{t('discord.members_hint')}</p>

            {loading && (
                <div className="dc-pair-wait pad">
                    <SabaSpinner size="sm" /> {t('discord.members_loading')}
                </div>
            )}
            {!loading && (members || []).length === 0 && <p className="muted pad">{t('discord.members_empty')}</p>}

            <ul className="dc-list">
                {list.map((m) => {
                    const restricted = P.isMemberRestricted(settings, node.guildId, m.id);
                    const blocked = P.blockedCount(settings, node.guildId, m.id);
                    return (
                        <li key={m.id}>
                            <button
                                type="button"
                                className="dc-list-row dc-list-btn"
                                onClick={() => onOpenMember(m.id)}
                            >
                                <Avatar name={memberName(m)} url={m.avatarUrl || m.avatar} />
                                <span className="dc-list-text">
                                    <span className="dc-list-name">{memberName(m)}</span>
                                    <span className="dc-list-sub">{m.username ? `@${m.username}` : m.id}</span>
                                </span>
                                <span className={clsx('dc-badge', restricted ? 'tone-warn' : 'tone-ok')}>
                                    {restricted
                                        ? t('discord.member_restricted', { count: blocked })
                                        : t('discord.member_all')}
                                </span>
                                <Icon name="chevronRight" size="sm" />
                            </button>
                        </li>
                    );
                })}
            </ul>
        </div>
    );
}

// ── 멤버 한 명: 인스턴스별 허용 명령 ──
function MemberDetail({ node, member, servers, settings, update, commandsFor, riskOf, onBack }) {
    const { t } = useTranslation('gui');
    const restricted = P.isMemberRestricted(settings, node.guildId, member.id);
    const visible = visibleServers(settings, node.guildId, servers);

    return (
        <div className="dc-tab">
            <button type="button" className="dc-crumb" onClick={onBack}>
                <Icon name="chevronLeft" size="sm" /> {t('discord.back_to_members')}
            </button>
            <div className="dc-member-head">
                <Avatar name={memberName(member)} url={member.avatarUrl || member.avatar} size={44} />
                <div>
                    <h2>{memberName(member)}</h2>
                    <span className="muted">{member.username ? `@${member.username}` : member.id}</span>
                </div>
            </div>

            <ChoiceCards
                name={`member-${member.id}`}
                value={restricted}
                onChange={(r) => update((s) => P.setMemberRestricted(s, node.guildId, member.id, r))}
                options={[false, true].map((r) => ({
                    value: r,
                    title: t(r ? 'discord.member_mode_pick' : 'discord.member_mode_all'),
                    desc: t(r ? 'discord.member_mode_pick_desc' : 'discord.member_mode_all_desc'),
                }))}
            />

            {restricted && visible.length === 0 && <p className="muted pad">{t('discord.member_no_instances')}</p>}
            {restricted &&
                visible.map((srv) => {
                    const cmds = commandsFor(srv.module);
                    const blocked = P.blockedCommands(settings, node.guildId, member.id, srv.id);
                    const allowedCount = cmds.filter((c) => !blocked.includes(c.name)).length;
                    const names = cmds.map((c) => c.name);
                    return (
                        <section key={srv.id} className="dc-card dc-perm-card">
                            <div className="dc-card-head">
                                <span className="dc-srv-icon">{initialOf(srv.name)}</span>
                                <div className="dc-list-text">
                                    <span className="dc-list-name">{srv.name}</span>
                                    <span className="dc-list-sub">
                                        {t('discord.allowed_of', { allowed: allowedCount, total: cmds.length })}
                                    </span>
                                </div>
                                {cmds.length > 0 && (
                                    <div className="dc-mini-actions">
                                        <button
                                            type="button"
                                            className="btn btn-ghost btn-sm"
                                            onClick={() =>
                                                update((s) =>
                                                    P.setMemberAllCommands(
                                                        s,
                                                        node.guildId,
                                                        member.id,
                                                        srv.id,
                                                        names,
                                                        true,
                                                    ),
                                                )
                                            }
                                        >
                                            {t('discord.allow_all')}
                                        </button>
                                        <button
                                            type="button"
                                            className="btn btn-ghost btn-sm"
                                            onClick={() =>
                                                update((s) =>
                                                    P.setMemberAllCommands(
                                                        s,
                                                        node.guildId,
                                                        member.id,
                                                        srv.id,
                                                        names,
                                                        false,
                                                    ),
                                                )
                                            }
                                        >
                                            {t('discord.block_all')}
                                        </button>
                                    </div>
                                )}
                            </div>
                            {cmds.length === 0 ? (
                                <p className="muted">{t('discord.no_commands')}</p>
                            ) : (
                                <div className="dc-cmd-grid">
                                    {cmds.map((c) => {
                                        const on = !blocked.includes(c.name);
                                        const risk = riskOf(srv.module, c.name);
                                        return (
                                            <button
                                                key={c.name}
                                                type="button"
                                                aria-pressed={on}
                                                title={c.description || c.name}
                                                className={clsx('dc-cmd', { on, danger: risk === 'danger' })}
                                                onClick={() =>
                                                    update((s) =>
                                                        P.toggleMemberCommand(
                                                            s,
                                                            node.guildId,
                                                            member.id,
                                                            srv.id,
                                                            c.name,
                                                        ),
                                                    )
                                                }
                                            >
                                                <Icon name={on ? 'check' : 'x'} size={14} />
                                                <span>{c.label}</span>
                                                {risk === 'danger' && <Icon name="alertTriangle" size={12} />}
                                            </button>
                                        );
                                    })}
                                </div>
                            )}
                        </section>
                    );
                })}
        </div>
    );
}

/**
 * 디스코드 서버(노드) 하나: 인스턴스 / 멤버 → 멤버별 명령.
 */
export function NodeView({
    node,
    nodes,
    servers,
    settings,
    update,
    updateAllowed,
    members,
    membersLoading,
    membersFailed,
    membersPartial,
    onRefreshMembers,
    memberId,
    onOpenMember,
    commandsFor,
    riskOf,
    saveState,
}) {
    const { t } = useTranslation('gui');
    const [tab, setTab] = useState('instances');
    const member = useMemo(() => (members || []).find((m) => m.id === memberId), [members, memberId]);
    const mode = P.instanceMode(settings, node.guildId);
    const visibleCount = visibleServers(settings, node.guildId, servers).length;
    const restrictedCount = Object.keys(P.nodeConfig(settings, node.guildId).memberPermissions || {}).length;
    const levels = permissionsAvailable(); // 데몬 P1 권한 레벨 (없으면 예전 명령별 차단)
    const activeTab = member && !levels ? 'members' : tab;

    return (
        <div className="dc-view">
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
                            {!levels && (
                                <>
                                    {' · '}
                                    {t('discord.node_sub_members', { count: restrictedCount })}
                                </>
                            )}
                        </span>
                    </div>
                </div>
                {saveState && (
                    <span className={clsx('dc-save-state', saveState)}>
                        {saveState === 'saving' ? t('discord.saving') : t('discord.saved')}
                    </span>
                )}
            </header>

            <Segmented
                ariaLabel={node.guildName}
                value={activeTab}
                onChange={(id) => {
                    setTab(id);
                    onOpenMember(null);
                }}
                items={[
                    { id: 'instances', label: t('discord.tab_instances'), icon: <Icon name="server" size="sm" /> },
                    { id: 'members', label: t('discord.tab_members'), icon: <Icon name="user" size="sm" /> },
                    ...(levels
                        ? [
                              {
                                  id: 'commands',
                                  label: t('discord.tab_commands'),
                                  icon: <Icon name="terminal" size="sm" />,
                              },
                              { id: 'levels', label: t('discord.tab_levels'), icon: <Icon name="key" size="sm" /> },
                          ]
                        : []),
                    ...(channelsAvailable()
                        ? [{ id: 'channels', label: t('discord.tab_channels'), icon: <Icon name="hash" size="sm" /> }]
                        : []),
                    ...(botSettingsAvailable()
                        ? [{ id: 'bot', label: t('discord.tab_bot'), icon: <Icon name="settings" size="sm" /> }]
                        : []),
                ]}
            />

            {membersFailed &&
                !membersLoading &&
                members?.length > 0 &&
                (activeTab === 'members' || activeTab === 'commands') && (
                    <p className="dc-field-error">
                        <Icon name="info" size="xs" /> {t('discord.members_refresh_failed')}
                    </p>
                )}
            {membersPartial &&
                !membersFailed &&
                !membersLoading &&
                (activeTab === 'members' || activeTab === 'commands') && (
                    <p className="dc-field-error">
                        <Icon name="info" size="xs" /> {t('discord.members_partial')}
                    </p>
                )}
            {activeTab === 'instances' && (
                <InstancesTab
                    node={node}
                    nodes={nodes}
                    servers={servers}
                    settings={settings}
                    update={(fn) => updateAllowed(node.guildId, fn)}
                />
            )}
            {activeTab === 'channels' && <ChannelsTab key={node.guildId} node={node} />}
            {activeTab === 'bot' && <BotSettingsTab key={node.guildId} node={node} />}
            {levels && (activeTab === 'members' || activeTab === 'commands' || activeTab === 'levels') && (
                <PermissionPanel
                    key={node.guildId}
                    tab={activeTab}
                    node={node}
                    servers={visibleServers(settings, node.guildId, servers)}
                    members={members}
                    membersLoading={membersLoading}
                    onRefreshMembers={onRefreshMembers}
                    commandsFor={commandsFor}
                    riskOf={riskOf}
                />
            )}
            {!levels && activeTab === 'members' && !member && (
                <MembersTab
                    node={node}
                    members={members}
                    loading={membersLoading}
                    settings={settings}
                    onRefresh={onRefreshMembers}
                    onOpenMember={onOpenMember}
                />
            )}
            {!levels && activeTab === 'members' && member && (
                <MemberDetail
                    node={node}
                    member={member}
                    servers={servers}
                    settings={settings}
                    update={update}
                    commandsFor={commandsFor}
                    riskOf={riskOf}
                    onBack={() => onOpenMember(null)}
                />
            )}
        </div>
    );
}
