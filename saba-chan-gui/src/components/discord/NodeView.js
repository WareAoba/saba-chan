import clsx from 'clsx';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { SabaToggle } from '../ui/SabaUI';
import { Segmented } from '../ui/StatusChip';
import { Avatar, initialOf } from './Avatar';
import { BotSettingsTab, botSettingsAvailable } from './BotSettingsTab';
import { ChatbotGuildTab } from './ChatbotGuildTab';
import { chatbotAvailable } from './ChatbotView';
import * as P from './nodePermissions';
import { ALL_INSTANCES } from './levels';
import { ModerationTab, moderationAvailable } from './ModerationTab';
import { PermissionPanel } from './PermissionsTab';

export { Avatar, initialOf };

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

/** 인스턴스의 모듈 아이콘 (서버 목록과 같다). 아이콘이 없는 모듈은 이름 첫 글자 */
function ModuleIcon({ module, name }) {
    return (
        <span className={clsx('dc-srv-icon', { 'has-img': !!module?.icon })}>
            {module?.icon ? <img src={module.icon} alt="" /> : initialOf(name)}
        </span>
    );
}

/** 노드에서 보이는 인스턴스 (제한 없음이면 전부) */
function visibleServers(settings, guildId, servers) {
    return servers.filter((s) => P.isInstanceAllowed(settings, guildId, s.id));
}

// ── 인스턴스 탭 ──
function InstancesTab({ node, nodes, servers, modules = [], settings, update, updateStartNotice, onOpen }) {
    const { t } = useTranslation('gui');
    const mode = P.instanceMode(settings, node.guildId);
    const nodeIds = nodes.map((n) => n.guildId);
    const nameOf = (id) => nodes.find((n) => n.guildId === id)?.guildName || id;
    const noticed = P.nodeConfig(settings, node.guildId).startNotice;

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

            {servers.length > 0 && <p className="dc-hint">{t('discord.start_notice_hint')}</p>}
            <div className="dc-toolbar">
                <span className="muted">{t('discord.instance_menu_hint')}</span>
                <button type="button" className="btn btn-ghost btn-sm" onClick={() => onOpen(ALL_INSTANCES)}>
                    <Icon name="key" size="sm" /> {t('discord.instance_menu_all')}
                </button>
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
                        return (
                            <li key={srv.id} className={clsx('dc-list-row', { dim: !allowed })}>
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
                                <button
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

/**
 * 인스턴스 메뉴: 레벨별 명령 계단(이 인스턴스 + 서버 전체 명령)과 멤버별 레벨을 따로 보인다.
 * `instance`가 ALL_INSTANCES면 모든 인스턴스를 한 계단에 모아 본다
 */
function InstanceMenu({ node, servers, instance, onBack, members, membersLoading, onRefreshMembers }) {
    const { t } = useTranslation('gui');
    const srv = servers.find((s) => s.id === instance);
    const title = srv ? srv.name : t('discord.perm_view_all');
    return (
        <div className="dc-tab">
            <button type="button" className="dc-crumb" onClick={onBack}>
                <Icon name="chevronLeft" size="sm" /> {t('discord.back_to_instances')}
            </button>
            <h2 className="dc-menu-title">
                {title}
                {srv?.module && <small className="muted"> · {srv.module}</small>}
            </h2>
            <PermissionPanel
                node={node}
                instance={srv ? srv.id : ALL_INSTANCES}
                members={members}
                membersLoading={membersLoading}
                onRefreshMembers={onRefreshMembers}
            />
        </div>
    );
}

/**
 * 디스코드 서버(노드) 하나: 인스턴스(→ 인스턴스 메뉴) / 말하는 사바쨩 / 노래하는 사바쨩 / 읽어주는 사바쨩 / 서버 관리 / 봇 설정.
 */
export function NodeView({
    modules = [],
    node,
    nodes,
    servers,
    settings,
    updateAllowed,
    updateStartNotice,
    members,
    membersLoading,
    membersFailed,
    membersPartial,
    onRefreshMembers,
    saveState,
    sampleModuleWord,
}) {
    const { t } = useTranslation('gui');
    const [tab, setTab] = useState('instances');
    const [instance, setInstance] = useState(null); // 연 인스턴스 메뉴 (없으면 목록)
    const mode = P.instanceMode(settings, node.guildId);
    const visibleCount = visibleServers(settings, node.guildId, servers).length;

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
                value={tab}
                onChange={(v) => {
                    setTab(v);
                    setInstance(null);
                }}
                items={[
                    { id: 'instances', label: t('discord.tab_instances'), icon: <Icon name="server" size="sm" /> },
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
                                  icon: <Icon name="volume" size="sm" />,
                              },
                          ]
                        : []),
                    ...(moderationAvailable()
                        ? [
                              {
                                  id: 'moderation',
                                  label: t('discord.tab_moderation'),
                                  icon: <Icon name="tool" size="sm" />,
                              },
                          ]
                        : []),
                    ...(botSettingsAvailable()
                        ? [{ id: 'bot', label: t('discord.tab_bot'), icon: <Icon name="settings" size="sm" /> }]
                        : []),
                ]}
            />

            {membersFailed && !membersLoading && members?.length > 0 && instance && (
                <p className="dc-field-error">
                    <Icon name="info" size="xs" /> {t('discord.members_refresh_failed')}
                </p>
            )}
            {membersPartial && !membersFailed && !membersLoading && instance && (
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
                />
            )}
            {tab === 'moderation' && (
                <ModerationTab
                    key={node.guildId}
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
