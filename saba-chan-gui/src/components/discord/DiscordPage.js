import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSwapAnimation } from '../../hooks/useSwapAnimation';
import { useWindowSize } from '../../hooks/useWindowSize';
import { useDiscordStore } from '../../stores/useDiscordStore';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';
import { SINGLE_COLUMN_MAX_WIDTH } from '../servers/ServersPage';
import { StatusChip } from '../ui/StatusChip';
import { ChatbotView, chatbotAvailable } from './ChatbotView';
import { ConnectionView } from './ConnectionView';
import { Avatar, NodeView } from './NodeView';
import * as P from './nodePermissions';
import { permissionsAvailable } from './PermissionsTab';
import { defaultPrefix, prefixProblems } from './prefixes';
import { TagInput } from './TagInput';
import { persistNodeToken, usePairing } from './usePairing';

export const DEFAULT_RELAY_URL = 'https://saba-chan.online';
const AUTOSAVE_MS = 600;

/** 짝짓기 전에 릴레이 주소(설정 화면에서 바꿨을 수 있다)를 config에 저장한다. 실패하면 false */
const saveRelayUrl = () => useDiscordStore.getState().saveConfig();

/** 연결 상태 머신: (checking →) no_host → connecting → connected | error */
export function cloudStateOf({ hostId, connected, connecting, error, loaded }) {
    // 데몬에서 설정을 아직 못 읽었으면 호스트가 비어 있어도 연결 전인지 알 수 없다
    if (!hostId) return loaded === false ? 'checking' : 'no_host';
    if (connected) return 'connected';
    if (error && !connecting) return 'error';
    return 'connecting';
}

function BotSettingsView({ prefixes, setPrefixes, nodes, sampleModuleWord }) {
    const { t } = useTranslation('gui');
    const fallback = defaultPrefix(t, nodes);
    // 고치는 즉시 스토어에 넣으면 자동 저장된다. 저장할 수 없는 목록은 넣지 않고 화면에만 둔다
    const [draft, setDraft] = useState(prefixes);
    useEffect(() => setDraft(prefixes), [prefixes]);
    const draftProblems = prefixProblems(draft, t);
    const change = (next) => {
        setDraft(next);
        if (prefixProblems(next, t).length === 0) setPrefixes(next);
    };
    const p = draft[0] || fallback;
    return (
        <div className="dc-view">
            <header className="dc-view-head">
                <h1>{t('discord.bot_title')}</h1>
                <p className="muted">{t('discord.bot_desc')}</p>
            </header>
            <section className="dc-card">
                <div className="dc-card-head">
                    <h2>{t('discord.prefix_title')}</h2>
                </div>
                <p className="dc-card-desc">{t('discord.prefix_desc')}</p>
                <TagInput
                    value={draft}
                    onChange={change}
                    placeholder={t('discord.prefix_default_placeholder', { prefix: fallback })}
                    ariaLabel={t('discord.prefix_title')}
                />
                {draft.length === 0 && (
                    <p className="dc-hint">{t('discord.prefix_default_hint', { prefix: fallback })}</p>
                )}
                {draftProblems.length > 0 && (
                    <p className="dc-field-error">
                        <Icon name="alertTriangle" size="xs" /> {draftProblems.join(' · ')}
                    </p>
                )}
            </section>
            <section className="dc-card">
                <div className="dc-card-head">
                    <h2>{t('discord.usage_title')}</h2>
                </div>
                <ul className="dc-usage">
                    <li>
                        <code>{p}</code>
                        <span>{t('discord.usage_help')}</span>
                    </li>
                    <li>
                        <code>{`${p} ${t('discord.usage_list_word')}`}</code>
                        <span>{t('discord.usage_list')}</span>
                    </li>
                    <li>
                        <code>{`${p} ${sampleModuleWord} ${t('discord.usage_start_word')}`}</code>
                        <span>{t('discord.usage_command')}</span>
                    </li>
                </ul>
            </section>
        </div>
    );
}

/**
 * 디스코드 페이지 (V1): 왼쪽 메뉴 | 오른쪽 화면. 서버 페이지와 같은 목록-상세 구조.
 * 메뉴: 연결 · 디스코드 서버(노드) 목록 · 명령어 단어 · 기본 설정 · AI 챗봇.
 * 노드 권한은 바꾸는 즉시 저장(잠깐 모아서)하고, 단어 편집은 저장 버튼으로 확정한다.
 */
export function DiscordPage({
    discordBotStatus,
    discordPrefixes = [],
    setDiscordPrefixes,
    discordCloudRelayUrl,
    discordCloudHostId,
    setDiscordCloudHostId,
    relayConnected,
    relayConnecting,
    handleStartDiscordBot,
    saveCurrentSettings,
    servers = [],
    modules = [],
    moduleAliasesPerModule,
    discordModuleAliases,
    nodeSettings,
    setNodeSettings,
    cloudNodes = [],
    setCloudNodes,
    cloudMembers = {},
    setCloudMembers,
    initialView,
}) {
    const { t } = useTranslation('gui');
    const relayUrl = discordCloudRelayUrl || DEFAULT_RELAY_URL;
    const { width } = useWindowSize();
    const singleColumn = width < SINGLE_COLUMN_MAX_WIDTH;
    const [view, setView] = useState(initialView || { kind: 'connection' });
    // 왼쪽에서 다른 화면을 고르면 오른쪽이 살짝 떠오르며 바뀐다
    const mainRef = useSwapAnimation(`${view.kind}:${view.id ?? ''}`);
    const [showMainNarrow, setShowMainNarrow] = useState(!!initialView);
    const [cloudError, setCloudError] = useState('');
    // 릴레이 확인이 연달아 실패한 이유 (노드 목록은 연결됐을 때만 읽으므로 그 오류만으로는 실패를 알 수 없다)
    const relayError = useDiscordStore((s) => s.relayError);
    const connectionError = cloudError || relayError;
    const agentReplaced = useDiscordStore((s) => s.discordAgentReplaced);
    const botConfigLoaded = useDiscordStore((s) => s._botConfigLoaded);
    const [membersLoading, setMembersLoading] = useState(null);
    const [membersFailed, setMembersFailed] = useState({});
    // 길드별 일부 목록 사유 (권한을 등록한 멤버만 받았을 때)
    const [membersPartial, setMembersPartial] = useState({});
    const [saveState, setSaveState] = useState(null);
    const saveTimer = useRef(null);

    // 서버 설정의 "명령어 단어" 링크 `{kind:'words', server}`: 노드 목록이 오면 그 서버를 쓰는
    // 첫 길드의 부르는 말 탭으로 바꾼다. 그동안(또는 그런 길드가 없으면) 연결 화면을 보인다
    useEffect(() => {
        if (view.kind !== 'words' || !cloudNodes?.length) return;
        const node = cloudNodes.find((n) => P.isInstanceAllowed(nodeSettings || {}, n.guildId, view.server));
        setView(node ? { kind: 'node', id: node.guildId, tab: 'words' } : { kind: 'connection' });
    }, [view, cloudNodes, nodeSettings]);

    useEffect(() => {
        if (initialView) {
            setView(initialView);
            setShowMainNarrow(true);
        }
    }, [initialView]);

    const cloudState = cloudStateOf({
        hostId: discordCloudHostId,
        connected: relayConnected,
        connecting: relayConnecting,
        error: connectionError,
        loaded: botConfigLoaded,
    });

    const go = (next) => {
        setView(next);
        setShowMainNarrow(true);
    };

    // App이 넘기는 setter는 렌더마다 새 함수라 effect 의존성에 넣으면 무한 반복된다 → ref로 든다
    const setters = useRef({});
    setters.current = { setCloudNodes, setCloudMembers };

    // 늦게 온 응답이 바뀐 연결(호스트·릴레이·연결 끊김)의 화면을 옛 목록으로 되돌리지 않게 한다
    const relayKey = `${discordCloudHostId}|${relayUrl}|${relayConnected}`;
    const relayKeyRef = useRef(relayKey);
    relayKeyRef.current = relayKey;
    // 멤버는 길드별로 받으므로 연결 상태가 잠깐 흔들려도 같은 호스트·릴레이의 답이면 쓴다
    const hostKeyRef = useRef('');
    hostKeyRef.current = `${discordCloudHostId}|${relayUrl}`;

    // ── 노드 목록 ──
    const loadNodes = useCallback(async () => {
        if (!discordCloudHostId) return;
        const key = relayKeyRef.current;
        setCloudError('');
        try {
            const data = await window.api.relayListHostNodes(discordCloudHostId, relayUrl);
            if (key !== relayKeyRef.current) return;
            if (data?.error) throw new Error(data.error);
            setters.current.setCloudNodes(Array.isArray(data) ? data : data?.nodes || []);
        } catch (e) {
            if (key !== relayKeyRef.current) return;
            setCloudError(e.message || t('errors.network_error'));
        }
    }, [discordCloudHostId, relayUrl, t]);

    useEffect(() => {
        if (discordCloudHostId && relayConnected) loadNodes();
    }, [discordCloudHostId, relayConnected, loadNodes]);

    const loadMembers = useCallback(
        async (guildId) => {
            const key = hostKeyRef.current;
            setMembersLoading(guildId);
            try {
                const data = await window.api.relayListNodeMembers(guildId, relayUrl);
                if (key !== hostKeyRef.current) return;
                if (data?.error) throw new Error(data.error);
                const members = Array.isArray(data) ? data : data?.members || [];
                setters.current.setCloudMembers((prev) => ({ ...prev, [guildId]: members }));
                setMembersFailed((prev) => ({ ...prev, [guildId]: false }));
                setMembersPartial((prev) => ({ ...prev, [guildId]: data?.partial ? data.reason || 'partial' : null }));
            } catch (e) {
                // 받아 둔 목록은 그대로 둔다 (실패를 빈 목록으로 덮지 않는다)
                console.warn('[Discord] members fetch failed:', e);
                if (key === hostKeyRef.current) setMembersFailed((prev) => ({ ...prev, [guildId]: true }));
            } finally {
                setMembersLoading((cur) => (cur === guildId ? null : cur));
            }
        },
        [relayUrl],
    );

    const nodeId = view.kind === 'node' ? view.id : null;
    // 노드를 열 때 한 번만 부른다 (캐시가 있으면 새로고침 버튼으로)
    // biome-ignore lint/correctness/useExhaustiveDependencies: cloudMembers가 바뀔 때마다 다시 부르지 않는다
    useEffect(() => {
        if (nodeId && !(cloudMembers[nodeId]?.length > 0)) loadMembers(nodeId);
    }, [nodeId, loadMembers]);

    // ── 노드 권한: 바로 반영, 잠깐 모아서 저장 ──
    const updateNodes = useCallback(
        (fn) => {
            setNodeSettings((prev) => fn(prev || {}));
            setSaveState('saving');
            clearTimeout(saveTimer.current);
            saveTimer.current = setTimeout(async () => {
                // 실패는 저장 쪽이 알린다. 저장됨 표시만 띄우지 않는다
                const ok = await saveCurrentSettings();
                setSaveState(ok === false ? null : 'saved');
            }, AUTOSAVE_MS);
        },
        [setNodeSettings, saveCurrentSettings],
    );
    useEffect(() => () => clearTimeout(saveTimer.current), []);

    // ── 길드가 쓸 서버 제한: 데몬 길드 API로 바로 저장 (bot-config 일괄 저장으로는 바뀌지 않는다) ──
    const updateAllowed = useCallback(
        async (guildId, fn) => {
            const next = P.nodeConfig(fn(nodeSettings || {}), guildId).allowedInstances;
            setSaveState('saving');
            try {
                const res = await window.api.guildAllowedInstancesSave(guildId, Array.isArray(next) ? next : null);
                if (!res || res.error || res.error_code) throw new Error(res?.error || res?.error_code || '-');
                useDiscordStore
                    .getState()
                    .applySavedNodeValue(guildId, 'allowedInstances', res.allowedInstances ?? null);
                setSaveState('saved');
            } catch (e) {
                setSaveState(null);
                safeShowToast(t('discord.allowed_save_failed', { error: e.message }), 'error', 5000);
            }
        },
        [nodeSettings, t],
    );

    // ── 페어링 ──
    const pairing = usePairing({
        relayUrl,
        beforeStart: saveRelayUrl,
        onClaimed: async (s) => {
            // 토큰을 저장하지 못했으면 호스트도 바꾸지 않는다 (토큰 없는 호스트가 저장되지 않게)
            if (s.nodeToken && window.api?.saveNodeToken && !(await persistNodeToken(s.nodeToken))) return false;
            if (s.hostId) setDiscordCloudHostId(s.hostId);
            await saveCurrentSettings();
            loadNodes();
        },
    });

    // 연결 해제: 화면만 비우면 데몬 에이전트가 계속 명령을 받는다 → 에이전트를 멈추고 노드 토큰을 지운다
    // (토큰이 남아 있으면 데몬이 다음 시작 때 에이전트를 다시 켠다)
    // 하나라도 실패하면 화면을 비우지 않는다 (비우면 끊긴 줄 알지만 명령은 계속 들어온다)
    const disconnect = async () => {
        const failure = (r) =>
            !r ? 'no response' : r.error || (r.ok === false || r.success === false ? 'failed' : null);
        for (const call of [window.api.discordBotStop, window.api.deleteNodeToken]) {
            let err;
            try {
                err = call ? failure(await call()) : 'unavailable';
            } catch (e) {
                err = e.message || String(e);
            }
            if (err) {
                safeShowToast(t('discord.disconnect_failed', { error: err }), 'error', 5000);
                return;
            }
        }
        pairing.reset();
        setDiscordCloudHostId('');
        setCloudError('');
        setCloudNodes([]);
        setCloudMembers({});
        setView({ kind: 'connection' });
        saveCurrentSettings();
    };

    // ── 봇 초대: 데몬이 릴레이의 초대 주소를 받아 온다 (GET /api/relay/bot-invite) ──
    const inviteBot = window.api?.relayBotInvite
        ? async () => {
              try {
                  await saveRelayUrl();
                  const res = await window.api.relayBotInvite(relayUrl);
                  const url = res?.url || res?.inviteUrl || res?.invite_url;
                  if (!url) throw new Error(res?.error || 'no invite url');
                  const opened = await window.api.shell?.openExternal?.(url);
                  if (opened?.error) throw new Error(opened.error);
              } catch (e) {
                  safeShowToast(t('discord.invite_failed', { error: e.message }), 'error', 4000);
              }
          }
        : null;

    // ── 명령 목록 / 위험도 ──
    const commandsFor = useCallback(
        (moduleName) =>
            Object.entries(moduleAliasesPerModule?.[moduleName]?.commands || {}).map(([name, info]) => ({
                name,
                label: info.label || name,
                description: info.description || '',
            })),
        [moduleAliasesPerModule],
    );
    const riskOf = useCallback(
        (moduleName, cmd) =>
            modules.find((m) => m.name === moduleName)?.commands?.fields?.find((f) => f.name === cmd)?.risk || null,
        [modules],
    );

    const settings = nodeSettings || {};
    const currentNode = cloudNodes.find((n) => n.guildId === nodeId);
    const sampleModule = modules[0]?.name;
    const sampleModuleWord =
        P.parseAliasList(discordModuleAliases?.[sampleModule])[0] ||
        moduleAliasesPerModule?.[sampleModule]?.module_aliases?.[0] ||
        sampleModule ||
        'minecraft';

    // 노드가 사라졌으면 연결 화면으로
    useEffect(() => {
        if (view.kind === 'node' && cloudNodes.length > 0 && !cloudNodes.some((n) => n.guildId === view.id)) {
            setView({ kind: 'connection' });
        }
    }, [view, cloudNodes]);

    const connTone = { connected: 'ok', connecting: 'warn', checking: 'warn', error: 'stop', no_host: 'idle' }[
        cloudState
    ];
    const connLabel = t(`discord.state_${cloudState}`);

    const navHidden = singleColumn && showMainNarrow;
    const mainHidden = singleColumn && !showMainNarrow;

    const navItem = (key, active, onClick, icon, label, sub, extra) => (
        <button
            key={key}
            type="button"
            className={clsx('server-row dc-nav-row', { selected: active })}
            onClick={onClick}
            aria-current={active ? 'page' : undefined}
        >
            <span className="server-row-icon">{icon}</span>
            <span className="server-row-text">
                <span className="server-row-name">{label}</span>
                {sub && <span className="server-row-sub">{sub}</span>}
            </span>
            {extra}
        </button>
    );

    return (
        <div className={clsx('page servers-page discord-page', { 'single-column': singleColumn })}>
            {!navHidden && (
                <aside className="server-list-pane dc-nav" aria-label={t('shell.nav_discord')}>
                    <div className="server-list dc-nav-list">
                        {navItem(
                            'conn',
                            view.kind === 'connection',
                            () => go({ kind: 'connection' }),
                            <span className={clsx('dc-brand-mark', { on: cloudState === 'connected' })}>
                                <Icon name={cloudState === 'connected' ? 'discordLogo' : 'discord'} size="sm" />
                            </span>,
                            t('discord.nav_connection'),
                            <StatusChip
                                tone={connTone}
                                pulse={cloudState === 'connecting' || cloudState === 'checking'}
                                label={connLabel}
                            />,
                        )}

                        <div className="dc-nav-group">
                            <span>{t('discord.nav_nodes')}</span>
                            {cloudState === 'connected' && (
                                <button
                                    type="button"
                                    className="icon-btn dc-nav-add"
                                    data-tooltip={t('discord.pair_start')}
                                    aria-label={t('discord.pair_start')}
                                    onClick={() => {
                                        go({ kind: 'connection' });
                                        if (!pairing.active) pairing.start();
                                    }}
                                >
                                    <Icon name="plus" size="sm" />
                                </button>
                            )}
                        </div>
                        {cloudState === 'connected' && cloudError && (
                            <p className="dc-nav-empty dc-field-error">
                                {t('discord.nodes_refresh_failed', { error: cloudError })}
                                <button type="button" className="btn btn-ghost btn-sm" onClick={loadNodes}>
                                    {t('discord.reload')}
                                </button>
                            </p>
                        )}
                        {cloudNodes.length === 0 && !(cloudState === 'connected' && cloudError) && (
                            <p className="dc-nav-empty">
                                {cloudState === 'connected'
                                    ? t('discord.nodes_empty')
                                    : t('discord.nodes_need_connect')}
                            </p>
                        )}
                        {cloudNodes.map((n) => {
                            const mode = P.instanceMode(settings, n.guildId);
                            const count = servers.filter((s) => P.isInstanceAllowed(settings, n.guildId, s.id)).length;
                            const restricted = Object.keys(settings[n.guildId]?.memberPermissions || {}).length;
                            return navItem(
                                n.guildId,
                                nodeId === n.guildId,
                                () => go({ kind: 'node', id: n.guildId }),
                                <Avatar name={n.guildName || n.guildId} url={n.iconUrl} size={40} />,
                                n.guildName || n.guildId,
                                <>
                                    <span>
                                        {mode === 'all'
                                            ? t('discord.node_sub_all')
                                            : t('discord.node_sub_pick', { count })}
                                    </span>
                                    {restricted > 0 && !permissionsAvailable() && (
                                        <>
                                            <span className="sep">·</span>
                                            <span>{t('discord.node_sub_members', { count: restricted })}</span>
                                        </>
                                    )}
                                </>,
                            );
                        })}

                        <div className="dc-nav-group">
                            <span>{t('discord.nav_bot')}</span>
                        </div>
                        {navItem(
                            'bot',
                            view.kind === 'bot',
                            () => go({ kind: 'bot' }),
                            <Icon name="settings" size="sm" />,
                            t('discord.nav_settings'),
                            <span>
                                {t('discord.nav_settings_sub', {
                                    prefix: discordPrefixes.length
                                        ? discordPrefixes.join(', ')
                                        : t('discord.prefix_default_label', { prefix: defaultPrefix(t, cloudNodes) }),
                                })}
                            </span>,
                        )}
                        {chatbotAvailable() &&
                            navItem(
                                'chatbot',
                                view.kind === 'chatbot',
                                () => go({ kind: 'chatbot' }),
                                <Icon name="messageSquare" size="sm" />,
                                t('chatbot.nav'),
                                <span>{t('chatbot.nav_sub')}</span>,
                            )}
                    </div>
                </aside>
            )}

            {!mainHidden && (
                <main className="server-detail-pane">
                    <div className="server-detail dc-main" ref={mainRef}>
                        {singleColumn && (
                            <button type="button" className="dc-crumb" onClick={() => setShowMainNarrow(false)}>
                                <Icon name="chevronLeft" size="sm" /> {t('discord.back_to_menu')}
                            </button>
                        )}
                        {(view.kind === 'connection' || view.kind === 'words') && (
                            <ConnectionView
                                cloudState={cloudState}
                                hostId={discordCloudHostId}
                                relayUrl={relayUrl}
                                cloudError={connectionError}
                                nodeCount={cloudNodes.length}
                                botStatus={discordBotStatus}
                                agentReplaced={agentReplaced}
                                pairing={pairing}
                                onStartPairing={pairing.start}
                                onCancelPairing={pairing.reset}
                                onReload={loadNodes}
                                onDisconnect={disconnect}
                                onStartAgent={handleStartDiscordBot}
                                nodes={cloudNodes}
                                onInviteBot={inviteBot}
                            />
                        )}
                        {view.kind === 'node' && currentNode && (
                            <NodeView
                                key={currentNode.guildId}
                                node={currentNode}
                                nodes={cloudNodes}
                                servers={servers}
                                settings={settings}
                                update={updateNodes}
                                updateAllowed={updateAllowed}
                                members={cloudMembers[currentNode.guildId]}
                                membersLoading={membersLoading === currentNode.guildId}
                                membersFailed={!!membersFailed[currentNode.guildId]}
                                membersPartial={membersPartial[currentNode.guildId] || null}
                                onRefreshMembers={() => loadMembers(currentNode.guildId)}
                                initialTab={view.tab}
                                memberId={view.member || null}
                                onOpenMember={(id) => setView({ kind: 'node', id: currentNode.guildId, member: id })}
                                commandsFor={commandsFor}
                                riskOf={riskOf}
                                saveState={saveState}
                            />
                        )}
                        {view.kind === 'bot' && (
                            <BotSettingsView
                                prefixes={discordPrefixes}
                                setPrefixes={setDiscordPrefixes}
                                nodes={cloudNodes}
                                sampleModuleWord={sampleModuleWord}
                            />
                        )}
                        {view.kind === 'chatbot' && chatbotAvailable() && <ChatbotView nodes={cloudNodes} />}
                    </div>
                </main>
            )}
        </div>
    );
}
