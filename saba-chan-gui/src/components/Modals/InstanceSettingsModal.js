import { ModalOverlay } from './Modals';
import clsx from 'clsx';
import { useCallback, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import './Modals.css';
import { Icon } from '../Icon';
import { SabaCheckbox } from '../ui/SabaUI';

/** 유저 목록 페이지당 표시 수 */
const USERS_PER_PAGE = 10;

/** 노드/유저 제한을 걸 수 있는 기본 라이프사이클 명령어 (module.toml 미정의 시 보강) */
const LIFECYCLE_COMMANDS = ['start', 'stop', 'restart', 'status'];

/**
 * InstanceSettingsModal — 디스코드 "서버별 설정" 팝업.
 *
 * 등록된 인스턴스마다:
 *  - 호출명 별명 오버라이드 (미설정 시 module.toml 기본)
 *  - 사용 가능한 디스코드 노드 체크 (미설정 시 모든 노드 허용)
 *  - 명령어별 별명 오버라이드 + 유저 차단(블랙리스트, 10명/페이지)
 *
 * 설정은 useDiscordStore.instanceSettings에 저장되어 bot-config.json으로
 * 자동 저장된다. 릴레이 서버로는 전송되지 않으며, 명령 수신 시 데몬
 * 디스패처가 실행/거부를 판정한다.
 */
function InstanceSettingsModal({
    onClose,
    servers,
    moduleAliasesPerModule,
    instanceSettings,
    setInstanceSettings,
    cloudNodes,
    cloudMembers,
    fetchNodeMembers,
    membersLoading,
    membersError,
}) {
    const { t } = useTranslation('gui');

    // ── 일시적 UI 상태 ──
    const [expandedInstance, setExpandedInstance] = useState(null);
    const [expandedCommand, setExpandedCommand] = useState({}); // `${instId}:${cmd}` → bool
    const [userPage, setUserPage] = useState({}); // `${instId}:${cmd}` → page(0-base)
    // 별명 입력은 파싱 결과로 되돌려 쓰면 타이핑(공백/쉼표)이 끊기므로 raw 텍스트를 별도 보관
    const [aliasText, setAliasText] = useState({}); // instId → text
    const [cmdAliasText, setCmdAliasText] = useState({}); // `${instId}:${cmd}` → text

    const getSetting = useCallback((instanceId) => instanceSettings?.[instanceId] || {}, [instanceSettings]);

    const patchSetting = useCallback(
        (instanceId, mutate) => {
            setInstanceSettings((prev) => {
                const cfg = { ...(prev?.[instanceId] || {}) };
                mutate(cfg);
                const next = { ...(prev || {}) };
                // 모든 키가 비면 인스턴스 항목 자체를 제거 (완전 기본값 상태)
                if (Object.keys(cfg).length === 0) {
                    delete next[instanceId];
                } else {
                    next[instanceId] = cfg;
                }
                return next;
            });
        },
        [setInstanceSettings],
    );

    // ── 별명 파싱 (쉼표/공백 구분) ──
    const parseAliases = (text) =>
        (text || '')
            .split(/[,\s]+/)
            .map((a) => a.trim())
            .filter((a) => a.length > 0);

    /** module.toml 기본 호출명 별명 */
    const defaultAliasesFor = useCallback(
        (moduleName) => moduleAliasesPerModule?.[moduleName]?.module_aliases || [],
        [moduleAliasesPerModule],
    );

    /** 모듈 명령어 목록 (module.toml + 라이프사이클 기본 명령 보강) */
    const commandsFor = useCallback(
        (moduleName) => {
            const modCmds = moduleAliasesPerModule?.[moduleName]?.commands || {};
            const out = [];
            const seen = new Set();
            for (const [name, info] of Object.entries(modCmds)) {
                seen.add(name);
                const defAliases = Array.isArray(info) ? info : info?.aliases || [];
                out.push({
                    name,
                    label: (info?.label) || name,
                    description: (info?.description) || '',
                    defaultAliases: defAliases,
                });
            }
            for (const name of LIFECYCLE_COMMANDS) {
                if (!seen.has(name)) {
                    out.push({ name, label: name, description: '', defaultAliases: [] });
                }
            }
            return out;
        },
        [moduleAliasesPerModule],
    );

    /** 인스턴스가 사용 가능한 노드들의 멤버 합집합 (id 중복 제거) */
    const membersForInstance = useCallback(
        (setting) => {
            const nodeIds = Array.isArray(setting.allowedNodes)
                ? setting.allowedNodes
                : (cloudNodes || []).map((n) => n.guildId);
            const seen = new Map();
            for (const gid of nodeIds) {
                for (const m of cloudMembers?.[gid] || []) {
                    if (!seen.has(m.id)) seen.set(m.id, m);
                }
            }
            return [...seen.values()];
        },
        [cloudNodes, cloudMembers],
    );

    /** 멤버 미로드 노드에 대해 로드 요청 (force=true면 캐시 무시 새로고침) */
    const loadMembersForInstance = useCallback(
        (setting, force = false) => {
            const nodeIds = Array.isArray(setting.allowedNodes)
                ? setting.allowedNodes
                : (cloudNodes || []).map((n) => n.guildId);
            for (const gid of nodeIds) {
                fetchNodeMembers?.(gid, force);
            }
        },
        [cloudNodes, fetchNodeMembers],
    );

    // ── 핸들러: 호출명 별명 ──
    const commitAliases = (instanceId, text) => {
        setAliasText((prev) => ({ ...prev, [instanceId]: text }));
        const arr = parseAliases(text);
        patchSetting(instanceId, (cfg) => {
            // 빈 입력 = 오버라이드 해제 → module.toml 기본값 사용
            if (arr.length === 0) delete cfg.aliases;
            else cfg.aliases = arr;
        });
    };

    const resetAliases = (instanceId) => {
        setAliasText((prev) => ({ ...prev, [instanceId]: '' }));
        patchSetting(instanceId, (cfg) => {
            delete cfg.aliases;
        });
    };

    // ── 핸들러: 명령어 별명 ──
    const commitCommandAliases = (instanceId, cmd, text) => {
        setCmdAliasText((prev) => ({ ...prev, [`${instanceId}:${cmd}`]: text }));
        const arr = parseAliases(text);
        patchSetting(instanceId, (cfg) => {
            const map = { ...(cfg.commandAliases || {}) };
            if (arr.length === 0) delete map[cmd];
            else map[cmd] = arr;
            if (Object.keys(map).length === 0) delete cfg.commandAliases;
            else cfg.commandAliases = map;
        });
    };

    // ── 핸들러: 노드 허용 ──
    const setAllowAllNodes = (instanceId, allowAll) => {
        patchSetting(instanceId, (cfg) => {
            if (allowAll) {
                // 미설정 = 모든 노드 허용 (기본값)
                delete cfg.allowedNodes;
            } else {
                // 제한 시작 — 현재 노드 전체를 체크 상태로 초기화해서
                // 토글 직후 갑자기 전부 차단되는 사고를 방지
                cfg.allowedNodes = (cloudNodes || []).map((n) => n.guildId);
            }
        });
    };

    const toggleNode = (instanceId, guildId) => {
        patchSetting(instanceId, (cfg) => {
            const arr = Array.isArray(cfg.allowedNodes) ? [...cfg.allowedNodes] : [];
            const idx = arr.indexOf(guildId);
            if (idx >= 0) arr.splice(idx, 1);
            else arr.push(guildId);
            cfg.allowedNodes = arr;
        });
    };

    // ── 핸들러: 유저 차단 (블랙리스트) ──
    const toggleUserBlocked = (instanceId, cmd, userId) => {
        patchSetting(instanceId, (cfg) => {
            const map = { ...(cfg.blockedUsers || {}) };
            const list = new Set(map[cmd] || []);
            if (list.has(userId)) list.delete(userId);
            else list.add(userId);
            if (list.size === 0) delete map[cmd];
            else map[cmd] = [...list];
            if (Object.keys(map).length === 0) delete cfg.blockedUsers;
            else cfg.blockedUsers = map;
        });
    };

    // ── 인스턴스 요약 배지 텍스트 ──
    const nodeSummary = (setting) => {
        if (!Array.isArray(setting.allowedNodes)) {
            return t('discord_modal.is_all_nodes_badge', { defaultValue: 'All nodes' });
        }
        return t('discord_modal.is_nodes_badge', {
            count: setting.allowedNodes.length,
            defaultValue: '{{count}} node(s)',
        });
    };

    const sortedServers = useMemo(() => servers || [], [servers]);

    const body = (
        <ModalOverlay  className="instance-settings-overlay" onClose={onClose}>
            <div className="instance-settings-modal" >
                <div className="discord-modal-header">
                    <div className="discord-modal-title">
                        <h2>🖥️ {t('discord_modal.is_title', { defaultValue: 'Per-instance settings' })}</h2>
                    </div>
                    <button type="button" className="discord-modal-close" onClick={onClose}>
                        <Icon name="close" size="sm" />
                    </button>
                </div>

                <div className="instance-settings-content">
                    <small className="discord-instance-select-desc">
                        {t('discord_modal.is_desc', {
                            defaultValue:
                                'Configure per-instance call names, allowed nodes and per-command user restrictions. Unset items fall back to defaults (all nodes / everyone allowed).',
                        })}
                    </small>

                    {sortedServers.length === 0 && (
                        <p className="discord-node-empty">{t('discord_modal.no_instances_available')}</p>
                    )}

                    {sortedServers.map((server) => {
                        const setting = getSetting(server.id);
                        const isExpanded = expandedInstance === server.id;
                        const defAliases = defaultAliasesFor(server.module);
                        const aliasValue =
                            aliasText[server.id] ?? (Array.isArray(setting.aliases) ? setting.aliases.join(', ') : '');
                        const usingDefaultAliases = !Array.isArray(setting.aliases);
                        const allowAll = !Array.isArray(setting.allowedNodes);
                        const cmds = commandsFor(server.module);
                        const members = membersForInstance(setting);

                        return (
                            <div key={server.id} className={clsx('discord-node-card', { expanded: isExpanded })}>
                                <button type="button"   
                                    className="discord-node-card-header"
                                    onClick={() => setExpandedInstance(isExpanded ? null : server.id)}
                                >
                                    <div className="discord-node-card-info">
                                        <span className="discord-node-guild-name">{server.name}</span>
                                        <span className="discord-node-guild-id">
                                            {server.module} · {nodeSummary(setting)}
                                        </span>
                                    </div>
                                    <Icon name={isExpanded ? 'chevronDown' : 'chevronRight'} size="sm" />
                                </button>

                                {isExpanded && (
                                    <div className="discord-node-card-body instance-settings-body">
                                        {/* ── 호출명 별명 ── */}
                                        <div className="instance-settings-section">
                                            <div className="instance-settings-section-header">
                                                <strong>
                                                    {t('discord_modal.is_aliases_label', {
                                                        defaultValue: 'Call name aliases',
                                                    })}
                                                </strong>
                                                {!usingDefaultAliases && (
                                                    <button type="button"
                                                        className="discord-instance-select-btn"
                                                        onClick={() => resetAliases(server.id)}
                                                    >
                                                        {t('discord_modal.is_reset_default', {
                                                            defaultValue: 'Use defaults',
                                                        })}
                                                    </button>
                                                )}
                                            </div>
                                            <input
                                                type="text"
                                                className="discord-input"
                                                value={aliasValue}
                                                placeholder={
                                                    defAliases.length > 0 ? defAliases.join(', ') : server.module
                                                }
                                                onChange={(e) => commitAliases(server.id, e.target.value)}
                                            />
                                            <small className="discord-instance-select-desc">
                                                {usingDefaultAliases
                                                    ? t('discord_modal.is_aliases_default_hint', {
                                                          defaultValue:
                                                              'Using defaults — leave empty to keep module defaults.',
                                                      })
                                                    : t('discord_modal.is_aliases_override_hint', {
                                                          defaultValue:
                                                              'Overriding defaults. Clear the field to restore module defaults.',
                                                      })}
                                            </small>
                                        </div>

                                        {/* ── 사용 가능 노드 ── */}
                                        <div className="instance-settings-section">
                                            <div className="instance-settings-section-header">
                                                <strong>
                                                    {t('discord_modal.is_nodes_label', {
                                                        defaultValue: 'Allowed nodes',
                                                    })}
                                                </strong>
                                            </div>
                                            <label className="discord-instance-select-item instance-settings-allow-all">
                                                <SabaCheckbox
                                                    checked={allowAll}
                                                    onChange={(checked) => setAllowAllNodes(server.id, checked)}
                                                />
                                                <span>
                                                    {t('discord_modal.is_all_nodes', {
                                                        defaultValue: 'Allow all nodes (default)',
                                                    })}
                                                </span>
                                            </label>
                                            {!allowAll && (
                                                <div className="discord-instance-select-list">
                                                    {(cloudNodes || []).length === 0 && (
                                                        <p className="discord-node-empty">
                                                            {t('discord_modal.is_no_nodes', {
                                                                defaultValue: 'No paired nodes.',
                                                            })}
                                                        </p>
                                                    )}
                                                    {(cloudNodes || []).map((node) => {
                                                        const checked =
                                                            setting.allowedNodes?.includes(node.guildId) ?? false;
                                                        return (
                                                            <label
                                                                key={node.guildId}
                                                                className={clsx('discord-instance-select-item', {
                                                                    selected: checked,
                                                                })}
                                                            >
                                                                <SabaCheckbox
                                                                    checked={checked}
                                                                    onChange={() => toggleNode(server.id, node.guildId)}
                                                                />
                                                                <div className="discord-instance-select-info">
                                                                    <span className="discord-instance-select-name">
                                                                        {node.guildName || node.guildId}
                                                                    </span>
                                                                    <span className="discord-instance-select-module">
                                                                        {node.guildId}
                                                                    </span>
                                                                </div>
                                                            </label>
                                                        );
                                                    })}
                                                    {Array.isArray(setting.allowedNodes) &&
                                                        setting.allowedNodes.length === 0 && (
                                                            <p className="discord-node-empty warning-text">
                                                                ⚠️{' '}
                                                                {t('discord_modal.is_no_nodes_warning', {
                                                                    defaultValue:
                                                                        'No nodes checked — this instance cannot be used from Discord.',
                                                                })}
                                                            </p>
                                                        )}
                                                </div>
                                            )}
                                        </div>

                                        {/* ── 명령어별 설정 (펼치기) ── */}
                                        <div className="instance-settings-section">
                                            <div className="instance-settings-section-header">
                                                <strong>
                                                    {t('discord_modal.is_commands_label', {
                                                        defaultValue: 'Per-command settings',
                                                    })}
                                                </strong>
                                            </div>
                                            {cmds.map((cmd) => {
                                                const key = `${server.id}:${cmd.name}`;
                                                const cmdExpanded = !!expandedCommand[key];
                                                const blocked = new Set(setting.blockedUsers?.[cmd.name] || []);
                                                const cmdAliasValue =
                                                    cmdAliasText[key] ??
                                                    (setting.commandAliases?.[cmd.name] || []).join(', ');
                                                const page = userPage[key] || 0;
                                                const totalPages = Math.max(
                                                    1,
                                                    Math.ceil(members.length / USERS_PER_PAGE),
                                                );
                                                const pageMembers = members.slice(
                                                    page * USERS_PER_PAGE,
                                                    (page + 1) * USERS_PER_PAGE,
                                                );

                                                return (
                                                    <div key={cmd.name} className="instance-settings-command">
                                                        <button type="button"   
                                                            className="instance-settings-command-header"
                                                            onClick={() => {
                                                                const opening = !cmdExpanded;
                                                                setExpandedCommand((prev) => ({
                                                                    ...prev,
                                                                    [key]: opening,
                                                                }));
                                                                if (opening) {
                                                                    loadMembersForInstance(setting);
                                                                }
                                                            }}
                                                        >
                                                            <Icon
                                                                name={cmdExpanded ? 'chevronDown' : 'chevronRight'}
                                                                size="sm"
                                                            />
                                                            <span className="instance-settings-command-name">
                                                                {cmd.label !== cmd.name
                                                                    ? `${cmd.name} (${cmd.label})`
                                                                    : cmd.name}
                                                            </span>
                                                            {blocked.size > 0 && (
                                                                <span className="instance-settings-blocked-badge">
                                                                    🚫 {blocked.size}
                                                                </span>
                                                            )}
                                                        </button>

                                                        {cmdExpanded && (
                                                            <div className="instance-settings-command-body">
                                                                {/* 명령어 별명 */}
                                                                <input
                                                                    type="text"
                                                                    className="discord-input"
                                                                    value={cmdAliasValue}
                                                                    placeholder={
                                                                        cmd.defaultAliases.length > 0
                                                                            ? cmd.defaultAliases.join(', ')
                                                                            : t(
                                                                                  'discord_modal.is_cmd_aliases_placeholder',
                                                                                  {
                                                                                      defaultValue:
                                                                                          'Command aliases (comma separated)',
                                                                                  },
                                                                              )
                                                                    }
                                                                    onChange={(e) =>
                                                                        commitCommandAliases(
                                                                            server.id,
                                                                            cmd.name,
                                                                            e.target.value,
                                                                        )
                                                                    }
                                                                />

                                                                {/* 유저 차단 목록 */}
                                                                <small className="discord-instance-select-desc">
                                                                    {t('discord_modal.is_users_hint', {
                                                                        defaultValue:
                                                                            'Unchecked users cannot run this command.',
                                                                    })}
                                                                </small>
                                                                {membersError && members.length > 0 && (
                                                                    <p className="discord-node-empty warning-text">
                                                                        ⚠️ {membersError}
                                                                    </p>
                                                                )}
                                                                {members.length === 0 ? (
                                                                    <div className="instance-settings-users-empty">
                                                                        <p className="discord-node-empty">
                                                                            {membersError
                                                                                ? `⚠️ ${membersError}`
                                                                                : t('discord_modal.is_users_empty', {
                                                                                      defaultValue:
                                                                                          'No member data loaded.',
                                                                                  })}
                                                                        </p>
                                                                        <button type="button"
                                                                            className="discord-instance-select-btn"
                                                                            disabled={membersLoading}
                                                                            onClick={() =>
                                                                                loadMembersForInstance(setting, true)
                                                                            }
                                                                        >
                                                                            🔄 {t('discord_modal.members_refresh')}
                                                                        </button>
                                                                    </div>
                                                                ) : (
                                                                    <>
                                                                        <div
                                                                            style={{
                                                                                display: 'flex',
                                                                                justifyContent: 'flex-end',
                                                                            }}
                                                                        >
                                                                            <button type="button"
                                                                                className="discord-instance-select-btn"
                                                                                disabled={membersLoading}
                                                                                onClick={() =>
                                                                                    loadMembersForInstance(
                                                                                        setting,
                                                                                        true,
                                                                                    )
                                                                                }
                                                                            >
                                                                                🔄 {t('discord_modal.members_refresh')}
                                                                            </button>
                                                                        </div>
                                                                        <div className="discord-member-perm-list">
                                                                            {pageMembers.map((member) => (
                                                                                <label
                                                                                    key={member.id}
                                                                                    className="discord-instance-select-item instance-settings-user-row"
                                                                                >
                                                                                    <SabaCheckbox
                                                                                        size="sm"
                                                                                        checked={
                                                                                            !blocked.has(member.id)
                                                                                        }
                                                                                        onChange={() =>
                                                                                            toggleUserBlocked(
                                                                                                server.id,
                                                                                                cmd.name,
                                                                                                member.id,
                                                                                            )
                                                                                        }
                                                                                    />
                                                                                    <div className="discord-instance-select-info">
                                                                                        <span className="discord-instance-select-name">
                                                                                            {member.displayName ||
                                                                                                member.username}
                                                                                        </span>
                                                                                        <span className="discord-instance-select-module">
                                                                                            {member.id}
                                                                                        </span>
                                                                                    </div>
                                                                                </label>
                                                                            ))}
                                                                        </div>
                                                                        {totalPages > 1 && (
                                                                            <div className="instance-settings-pagination">
                                                                                <button type="button"
                                                                                    className="discord-instance-select-btn"
                                                                                    disabled={page === 0}
                                                                                    onClick={() =>
                                                                                        setUserPage((prev) => ({
                                                                                            ...prev,
                                                                                            [key]: page - 1,
                                                                                        }))
                                                                                    }
                                                                                >
                                                                                    ←
                                                                                </button>
                                                                                <span className="instance-settings-page-info">
                                                                                    {page + 1} / {totalPages}
                                                                                </span>
                                                                                <button type="button"
                                                                                    className="discord-instance-select-btn"
                                                                                    disabled={page >= totalPages - 1}
                                                                                    onClick={() =>
                                                                                        setUserPage((prev) => ({
                                                                                            ...prev,
                                                                                            [key]: page + 1,
                                                                                        }))
                                                                                    }
                                                                                >
                                                                                    →
                                                                                </button>
                                                                            </div>
                                                                        )}
                                                                    </>
                                                                )}
                                                            </div>
                                                        )}
                                                    </div>
                                                );
                                            })}
                                        </div>
                                    </div>
                                )}
                            </div>
                        );
                    })}

                    <p className="instance-settings-autosave-hint">
                        💾 {t('discord_modal.is_autosave_hint', { defaultValue: 'Changes are saved automatically.' })}
                    </p>
                </div>
            </div>
        </ModalOverlay>
    );

    return createPortal(body, document.body);
}

export default InstanceSettingsModal;
