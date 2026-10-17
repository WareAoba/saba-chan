import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
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
import { usePairing } from './usePairing';
import { WordsView } from './WordsView';

export const DEFAULT_RELAY_URL = 'https://saba-chan.online';
const AUTOSAVE_MS = 600;

/** 짝짓기 전에 릴레이 주소(설정 화면에서 바꿨을 수 있다)를 config에 저장한다. 실패하면 false */
const saveRelayUrl = () => useDiscordStore.getState().saveConfig();

/** 연결 상태 머신: no_host → connecting → connected | error */
export function cloudStateOf({ hostId, connected, connecting, error }) {
    if (!hostId) return 'no_host';
    if (connected) return 'connected';
    if (error && !connecting) return 'error';
    return 'connecting';
}

function BotSettingsView({ prefix, setPrefix, onSave, sampleModuleWord }) {
    const { t } = useTranslation('gui');
    const [value, setValue] = useState(prefix);
    useEffect(() => setValue(prefix), [prefix]);
    const commit = () => {
        const v = value.trim();
        if (!v || v === prefix) {
            setValue(prefix);
            return;
        }
        setPrefix(v);
        onSave();
    };
    const p = value.trim() || prefix || '!saba';
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
                <input
                    className="input dc-prefix-input"
                    value={value}
                    aria-label={t('discord.prefix_title')}
                    placeholder="!saba"
                    onChange={(e) => setValue(e.target.value.replace(/\s/g, ''))}
                    onBlur={commit}
                    onKeyDown={(e) => e.key === 'Enter' && e.currentTarget.blur()}
                />
                {!value.trim() && <p className="dc-field-error">{t('discord.prefix_required')}</p>}
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
    discordPrefix,
    setDiscordPrefix,
    discordCloudRelayUrl,
    discordCloudHostId,
    setDiscordCloudHostId,
    relayConnected,
    relayConnecting,
    handleStartDiscordBot,
    handleStopDiscordBot,
    saveCurrentSettings,
    servers = [],
    modules = [],
    moduleAliasesPerModule,
    discordModuleAliases,
    discordCommandAliases,
    setDiscordModuleAliases,
    setDiscordCommandAliases,
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
    const [showMainNarrow, setShowMainNarrow] = useState(!!initialView);
    const [cloudError, setCloudError] = useState('');
    const [membersLoading, setMembersLoading] = useState(null);
    const [saveState, setSaveState] = useState(null);
    const saveTimer = useRef(null);

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
        error: cloudError,
    });

    const go = (next) => {
        setView(next);
        setShowMainNarrow(true);
    };

    // App이 넘기는 setter는 렌더마다 새 함수라 effect 의존성에 넣으면 무한 반복된다 → ref로 든다
    const setters = useRef({});
    setters.current = { setCloudNodes, setCloudMembers };

    // ── 노드 목록 ──
    const loadNodes = useCallback(async () => {
        if (!discordCloudHostId) return;
        setCloudError('');
        try {
            const data = await window.api.relayListHostNodes(discordCloudHostId, relayUrl);
            if (data?.error) throw new Error(data.error);
            setters.current.setCloudNodes(Array.isArray(data) ? data : data?.nodes || []);
        } catch (e) {
            setCloudError(e.message || t('errors.network_error'));
        }
    }, [discordCloudHostId, relayUrl, t]);

    useEffect(() => {
        if (discordCloudHostId && relayConnected) loadNodes();
    }, [discordCloudHostId, relayConnected, loadNodes]);

    const loadMembers = useCallback(
        async (guildId) => {
            setMembersLoading(guildId);
            try {
                const data = await window.api.relayListNodeMembers(guildId, relayUrl);
                const members = Array.isArray(data) ? data : data?.members || [];
                setters.current.setCloudMembers((prev) => ({ ...prev, [guildId]: members }));
            } catch (e) {
                console.warn('[Discord] members fetch failed:', e);
            } finally {
                setMembersLoading(null);
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
                await saveCurrentSettings();
                setSaveState('saved');
            }, AUTOSAVE_MS);
        },
        [setNodeSettings, saveCurrentSettings],
    );
    useEffect(() => () => clearTimeout(saveTimer.current), []);

    // ── 페어링 ──
    const pairing = usePairing({
        relayUrl,
        beforeStart: saveRelayUrl,
        onClaimed: async (s) => {
            if (s.hostId) setDiscordCloudHostId(s.hostId);
            if (s.nodeToken && window.api?.saveNodeToken) {
                try {
                    await window.api.saveNodeToken(s.nodeToken);
                } catch (e) {
                    console.error('[Pairing] node token save failed:', e);
                }
            }
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

    // 인스턴스 별명은 디스코드 페이지에서만 쓰므로 스토어 대신 여기서 bot-config를 직접 읽는다
    const [instanceAliases, setInstanceAliases] = useState({});
    useEffect(() => {
        let alive = true;
        Promise.resolve(window.api?.botConfigLoad?.())
            .then((cfg) => alive && setInstanceAliases(cfg?.instanceAliases || {}))
            .catch(() => {});
        return () => {
            alive = false;
        };
    }, []);

    // ── 명령어 단어 저장 (bot-config의 별명 키만 보낸다; 데몬이 최상위 키 단위로 병합) ──
    const persistAliases = async (mutate, okMsg) => {
        try {
            const current = (await window.api.botConfigLoad()) || {};
            const moduleAliases = { ...(current.moduleAliases || {}) };
            const commandAliases = { ...(current.commandAliases || {}) };
            const instanceAliases = { ...(current.instanceAliases || {}) };
            mutate(moduleAliases, commandAliases, instanceAliases);
            const res = await window.api.botConfigSave({
                prefix: current.prefix || discordPrefix || '!saba',
                moduleAliases,
                commandAliases,
                instanceAliases,
            });
            if (res?.error) throw new Error(res.error);
            setDiscordModuleAliases(moduleAliases);
            setDiscordCommandAliases(commandAliases);
            setInstanceAliases(instanceAliases);
            safeShowToast(okMsg, 'success', 2500);
        } catch (e) {
            safeShowToast(t('discord.words_save_failed', { error: e.message }), 'error', 4000);
        }
    };
    const saveWords = (moduleName, draft) =>
        persistAliases((ma, ca, ia) => {
            for (const [id, words] of Object.entries(draft.instances || {})) {
                if (words.length) ia[id] = words.join(',');
                else delete ia[id];
            }
            ma[moduleName] = draft.moduleAliases.join(',');
            ca[moduleName] = Object.fromEntries(
                Object.entries(draft.commands).map(([cmd, d]) => [cmd, d.aliases.join(',')]),
            );
        }, t('discord.words_saved'));
    const resetWords = (moduleName) =>
        persistAliases((ma, ca, ia) => {
            delete ma[moduleName];
            delete ca[moduleName];
            for (const s of servers) if (s.module === moduleName) delete ia[s.id];
        }, t('discord.words_reset_done'));

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

    const connTone = { connected: 'ok', connecting: 'warn', error: 'stop', no_host: 'idle' }[cloudState];
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
                            <Icon name="plug" size="sm" />,
                            t('discord.nav_connection'),
                            <StatusChip tone={connTone} pulse={cloudState === 'connecting'} label={connLabel} />,
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
                        {cloudNodes.length === 0 && (
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
                                    {restricted > 0 && (
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
                            'words',
                            view.kind === 'words',
                            () => go({ kind: 'words', module: view.module }),
                            <Icon name="hash" size="sm" />,
                            t('discord.nav_words'),
                            <span>{t('discord.nav_words_sub', { count: modules.length })}</span>,
                        )}
                        {navItem(
                            'bot',
                            view.kind === 'bot',
                            () => go({ kind: 'bot' }),
                            <Icon name="settings" size="sm" />,
                            t('discord.nav_settings'),
                            <span>{t('discord.nav_settings_sub', { prefix: discordPrefix || '—' })}</span>,
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
                    <div className="server-detail dc-main">
                        {singleColumn && (
                            <button type="button" className="dc-crumb" onClick={() => setShowMainNarrow(false)}>
                                <Icon name="chevronLeft" size="sm" /> {t('discord.back_to_menu')}
                            </button>
                        )}
                        {view.kind === 'connection' && (
                            <ConnectionView
                                cloudState={cloudState}
                                hostId={discordCloudHostId}
                                relayUrl={relayUrl}
                                cloudError={cloudError}
                                nodeCount={cloudNodes.length}
                                botStatus={discordBotStatus}
                                pairing={pairing}
                                onStartPairing={pairing.start}
                                onCancelPairing={pairing.reset}
                                onReload={loadNodes}
                                onDisconnect={disconnect}
                                onStartAgent={handleStartDiscordBot}
                                onStopAgent={handleStopDiscordBot}
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
                                members={cloudMembers[currentNode.guildId]}
                                membersLoading={membersLoading === currentNode.guildId}
                                onRefreshMembers={() => loadMembers(currentNode.guildId)}
                                memberId={view.member || null}
                                onOpenMember={(id) => setView({ kind: 'node', id: currentNode.guildId, member: id })}
                                commandsFor={commandsFor}
                                riskOf={riskOf}
                                saveState={saveState}
                            />
                        )}
                        {view.kind === 'words' && (
                            <WordsView
                                modules={modules}
                                moduleName={view.module}
                                onSelectModule={(m) => setView({ kind: 'words', module: m })}
                                prefix={discordPrefix}
                                moduleAliasesPerModule={moduleAliasesPerModule}
                                savedModuleAliases={discordModuleAliases}
                                savedCommandAliases={discordCommandAliases}
                                servers={servers}
                                savedInstanceAliases={instanceAliases}
                                onSave={saveWords}
                                onReset={resetWords}
                            />
                        )}
                        {view.kind === 'bot' && (
                            <BotSettingsView
                                prefix={discordPrefix}
                                setPrefix={setDiscordPrefix}
                                onSave={saveCurrentSettings}
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
