import clsx from 'clsx';
import type { TFunction } from 'i18next';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { type BackupNotice, type DmNoticeKind, useDiscordStore } from '../../stores/useDiscordStore';
import { useUIStore } from '../../stores/useUIStore';
import { safeShowToast } from '../../utils/helpers';
import { CliText } from '../CliLine';
import { Icon } from '../Icon';
import '../ui/SabaArt.css';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { useContextMenu } from '../ui/ContextMenu';
import { Segmented, StatusChip } from '../ui/StatusChip';
import type { HeartbeatReject } from '../../types/daemon';
import type { DiscordNode } from './types';
import type { usePairing } from './usePairing';
import { failed } from '../../utils/apiResult';

type Pairing = ReturnType<typeof usePairing>;

interface SleepAllButtonProps {
    nodes?: DiscordNode[];
    onAsleepChange?: (asleep: boolean) => void;
}

/**
 * 재우기: 연결은 그대로 두고 모든 디스코드 서버의 잠자기(데몬 discord settings `sleeping`)를 켠다.
 * 모두 잠들어 있으면 깨우기로 바뀐다. 일부만 바뀐 채 실패하면 방금 하려던 쪽을 다시 누를 수 있게 둔다.
 */
function SleepAllButton({ nodes, onAsleepChange }: SleepAllButtonProps) {
    const { t } = useTranslation('gui');
    const ids = (nodes || []).map((n) => n.guildId).join(',');
    const [asleep, setAsleep] = useState<boolean | null>(null); // null = 아직 모름
    const [busy, setBusy] = useState(false);
    // 늦게 온 옛 길드 목록의 응답은 버린다
    const generation = useRef(0);
    // 일부만 잠든 상태에서 보여줄 쪽: 마지막으로 시도한 동작 (없으면 재우기)
    const lastAttempt = useRef<'sleep' | 'wake' | null>(null);

    const load = useCallback(async () => {
        const gen = ++generation.current;
        const list = ids ? ids.split(',') : [];
        if (!list.length || !window.api?.discordSettingsGet) return setAsleep(false);
        const res = await Promise.all(list.map((g) => window.api.discordSettingsGet(g).catch(() => null)));
        if (gen !== generation.current) return;
        const sleepingCount = res.filter((r) => !failed(r) && r?.sleeping === true).length;
        if (sleepingCount === list.length) setAsleep(true);
        else if (sleepingCount === 0) setAsleep(false);
        else setAsleep(lastAttempt.current === 'wake');
    }, [ids]);
    useEffect(() => {
        lastAttempt.current = null;
        load();
    }, [load]);
    useEffect(() => {
        onAsleepChange?.(asleep === true);
    }, [asleep, onAsleepChange]);

    const toggle = async () => {
        const sleeping = !asleep;
        lastAttempt.current = sleeping ? 'sleep' : 'wake';
        setBusy(true);
        const list = ids.split(',');
        const res = await Promise.all(
            list.map((g) =>
                window.api.discordSettingsSet(g, { sleeping }).catch((e) => ({ error: (e as Error).message })),
            ),
        );
        const bad = res.filter(failed).length;
        if (bad) safeShowToast(t('discord.sleep_failed', { count: bad }), 'error', 6000);
        await load();
        setBusy(false);
    };

    const disabled = busy || asleep === null || !ids;
    return asleep ? (
        <button type="button" className="btn btn-primary" onClick={toggle} disabled={disabled}>
            <Icon name="sun" size="sm" /> {t('discord.agent_wake')}
        </button>
    ) : (
        <button type="button" className="btn btn-ghost" onClick={toggle} disabled={disabled}>
            <Icon name="zzz" size="sm" /> {t('discord.agent_stop')}
        </button>
    );
}

function CopyButton({ text, label }: { text: string; label: string }) {
    const { t } = useTranslation('gui');
    const [copied, setCopied] = useState(false);
    return (
        <button
            type="button"
            className="btn btn-ghost btn-sm"
            onClick={() =>
                navigator.clipboard?.writeText(text).then(() => {
                    setCopied(true);
                    setTimeout(() => setCopied(false), 1500);
                })
            }
        >
            <Icon name={copied ? 'check' : 'copy'} size="sm" /> {copied ? t('discord.copied') : label}
        </button>
    );
}

const PAIR_FAILURE_TEXT: Record<string, string> = {
    expired: 'discord.pair_expired',
    unsaved: 'discord.pair_not_saved',
    error: 'discord.pair_error',
};

interface PairingCardProps {
    pairing: Pairing;
    onCancel: () => void;
}

/** 페어링 진행 카드: 코드 → 디스코드에서 입력 → 자동 확인 */
export function PairingCard({ pairing, onCancel }: PairingCardProps) {
    const { t } = useTranslation('gui');
    const command = t('discord.pair_command', { code: pairing.code });

    if (pairing.status === 'issuing') {
        return (
            <div className="dc-card dc-pair">
                <div className="dc-pair-wait">
                    <SabaSpinner size="sm" /> {t('discord.pair_issuing')}
                </div>
            </div>
        );
    }
    if (pairing.status === 'success') {
        return (
            <div className="dc-card dc-pair tone-ok">
                <div className="dc-pair-result">
                    <Icon name="checkCircle" size="lg" />
                    <strong>{t('discord.pair_success')}</strong>
                </div>
            </div>
        );
    }
    if (pairing.status === 'expired' || pairing.status === 'unsaved' || pairing.status === 'error') {
        return (
            <div className="dc-card dc-pair tone-warn">
                <div className="dc-pair-result">
                    <Icon name="alertTriangle" size="lg" />
                    <strong>
                        {t(
                            pairing.errorCode === 'invalid_relay_url'
                                ? 'discord.invalid_relay_url'
                                : PAIR_FAILURE_TEXT[pairing.status],
                        )}
                    </strong>
                </div>
                <div className="dc-row-actions">
                    <button type="button" className="btn btn-ghost" onClick={onCancel}>
                        {t('discord.cancel')}
                    </button>
                    <button type="button" className="btn btn-primary" onClick={pairing.start}>
                        <Icon name="refresh" size="sm" /> {t('discord.pair_again')}
                    </button>
                </div>
            </div>
        );
    }

    const minutes = Math.floor(pairing.remaining / 60);
    const seconds = String(pairing.remaining % 60).padStart(2, '0');
    return (
        <div className="dc-card dc-pair">
            <ol className="dc-steps">
                <li>
                    <span className="dc-step-num">1</span>
                    <div className="dc-step-body">
                        <span className="dc-step-title">{t('discord.pair_step_code')}</span>
                        <div className="dc-code-row">
                            <span className="dc-code">{pairing.code}</span>
                        </div>
                    </div>
                </li>
                <li>
                    <span className="dc-step-num">2</span>
                    <div className="dc-step-body">
                        <span className="dc-step-title">{t('discord.pair_step_command')}</span>
                        <div className="dc-command-row">
                            <code>{command}</code>
                            <CopyButton text={command} label={t('discord.copy')} />
                        </div>
                    </div>
                </li>
                <li>
                    <span className="dc-step-num">3</span>
                    <div className="dc-step-body">
                        <span className="dc-step-title">{t('discord.pair_step_wait')}</span>
                        <div className="dc-pair-wait">
                            <SabaSpinner size="sm" />
                            <span>{t('discord.pair_waiting')}</span>
                            <span className="dc-timer">
                                {minutes}:{seconds}
                            </span>
                        </div>
                    </div>
                </li>
            </ol>
            {/* 디스코드의 "다시 연결할 PC" 고르기에서 다른 PC를 고르면 그 PC의 토큰이 바뀌어 끊긴다 */}
            <div className="notice tone-warn">
                <Icon name="info" size="sm" />
                <span>{t('discord.pair_pick_this_pc')}</span>
            </div>
            <div className="dc-row-actions">
                {/* 같은 일을 하는 CLI (J1-c) */}
                <CliText line="saba-chan-cli bot pair" />
                <button type="button" className="btn btn-ghost" onClick={onCancel}>
                    {t('discord.cancel')}
                </button>
            </div>
        </div>
    );
}

/** 흐름 상자 사이 이음줄: on = 초록 점선, off = 없음, fail = 빨간 점선 + X */
function ChainLink({ state }: { state: 'on' | 'off' | 'fail' }) {
    return (
        <li className={clsx('dc-chain-link', state)} aria-hidden="true">
            {state === 'fail' && (
                <span className="dc-chain-x">
                    <Icon name="x" size={12} />
                </span>
            )}
        </li>
    );
}

/** 릴레이 주소에서 호스트 이름만 (긴 URL이 흐름 칸을 넘치지 않게) */
function relayHost(url: string) {
    try {
        return new URL(url).host;
    } catch {
        return url || '-';
    }
}

interface ConnectionViewProps {
    cloudState: string;
    hostId: string;
    relayUrl: string;
    cloudError?: string;
    nodeCount: number;
    botStatus: string;
    agentReplaced?: boolean;
    agentRelayUrlInvalid?: boolean;
    agentMinVersion?: string | null;
    pairing: Pairing;
    onStartPairing: () => void;
    onCancelPairing: () => void;
    onReload: () => void;
    onDisconnect: () => void;
    onStartAgent: () => void;
    nodes?: DiscordNode[];
    onInviteBot?: (() => void) | null;
}

// 릴레이가 개수 상한으로 거절하는 상태 보고의 칸 → 문구 이름 (`discord.heartbeat_what_<이름>`)
const HEARTBEAT_COUNTS: Record<string, string> = {
    'metadata.servers': 'servers',
    'metadata.serverWords': 'servers',
    'metadata.moduleDetails': 'modules',
    'metadata.allowedInstances': 'guilds',
    'metadata.startNotice': 'guilds',
    'metadata.fullNotice': 'guilds',
    'metadata.permissions': 'guilds',
};

/** 거절 이유 한 줄: 아는 개수 상한이면 지금 값과 상한, 아니면 맞지 않은 칸 이름 */
function heartbeatRejectText(t: TFunction, reason: HeartbeatReject) {
    const what = HEARTBEAT_COUNTS[reason.field];
    if (reason.code === 'TOO_BIG' && what && reason.max != null && reason.actual != null)
        return t('discord.heartbeat_rejected_too_big', { what: t(`discord.heartbeat_what_${what}`), actual: reason.actual, max: reason.max });
    return t('discord.heartbeat_rejected_invalid', { what: reason.field });
}

/**
 * 연결 화면: 아직 연결 안 함 → 시작 안내 / 연결됨 → 연결 정보·에이전트·해제.
 */
export function ConnectionView({
    cloudState,
    hostId,
    relayUrl,
    cloudError,
    nodeCount,
    botStatus,
    agentReplaced,
    agentRelayUrlInvalid,
    agentMinVersion,
    pairing,
    onStartPairing,
    onCancelPairing,
    onReload,
    onDisconnect,
    onStartAgent,
    nodes,
    onInviteBot,
}: ConnectionViewProps) {
    const { t } = useTranslation('gui');
    const [confirmDisconnect, setConfirmDisconnect] = useState(false);
    const newGuildInstances = useDiscordStore((s) => s.discordNewGuildInstances);
    const reconnecting = useDiscordStore((s) => s.discordReconnecting);
    // 릴레이 실패 종류: 토큰 거절이면 이 PC를 다시 연결하게 하고, 요청 한도면 곧 다시 붙는다고 알린다
    const relayErrorKind = useDiscordStore((s) => s.relayErrorKind);
    // 릴레이가 이 PC의 상태 보고를 검증에서 거절했다 (이대로면 디스코드에서 오프라인이 된다)
    const heartbeatReject = useDiscordStore((s) => s.discordHeartbeatReject);
    const openMenu = useContextMenu();
    // 이 카드의 "다시 연결하기"·⋯ 메뉴로 시작한 페어링은 연결 카드 안에서 보인다
    const [pairInConn, setPairInConn] = useState(false);
    const infoRef = useRef<HTMLElement>(null);
    useEffect(() => {
        if (!pairing.active) setPairInConn(false);
    }, [pairing.active]);
    // SleepAllButton이 알려 주는 "모든 서버를 재웠는지"
    const [asleep, setAsleep] = useState(false);

    // 데몬에서 저장된 연결을 아직 읽지 못했다: 연결 버튼을 보이면 연결이 없는 줄 알고 다시 짝지을 수 있다
    if (cloudState === 'checking') {
        return (
            <div className="dc-view">
                <div className="dc-hero">
                    <div className="dc-hero-icon">
                        <Icon name="discord" size="xl" />
                    </div>
                    <h1>{t('discord.state_checking')}</h1>
                    <p>{t('discord.checking_body')}</p>
                </div>
            </div>
        );
    }

    if (cloudState === 'no_host') {
        return (
            <div className="dc-view">
                <div className="dc-hero">
                    <div className="dc-hero-icon">
                        <Icon name="discord" size="xl" />
                    </div>
                    <h1>{t('discord.connect_title')}</h1>
                    <p>{t('discord.connect_body')}</p>
                    {!pairing.active && (
                        <div className="dc-hero-actions">
                            {onInviteBot && (
                                <button
                                    data-spot="discord.invite"
                                    type="button"
                                    className="btn btn-ghost btn-lg"
                                    onClick={onInviteBot}
                                >
                                    <Icon name="externalLink" size="sm" /> {t('discord.invite_bot')}
                                </button>
                            )}
                            <button
                                data-spot="discord.pair"
                                type="button"
                                className="btn btn-primary btn-lg"
                                onClick={onStartPairing}
                            >
                                <Icon name="plug" size="sm" /> {t('discord.pair_start')}
                            </button>
                        </div>
                    )}
                    {onInviteBot && !pairing.active && <p className="dc-hero-hint muted">{t('discord.invite_hint')}</p>}
                </div>
                {pairing.active && <PairingCard pairing={pairing} onCancel={onCancelPairing} />}
            </div>
        );
    }

    const connected = cloudState === 'connected';
    const agentRunning = botStatus === 'running' || botStatus === 'connecting';
    // 릴레이가 이 앱 버전을 거절하면 세션이 살아 있어도 명령이 오지 않는다
    const refused = !!agentMinVersion;
    // 모든 서버를 재웠으면 명령이 흐르지 않으니 사바쨩 쪽 흐름을 끈다
    const flowing = agentRunning && connected && !asleep && !refused;
    // 릴레이에 닿지 못한 것은 "실패": 끊긴 자리를 빨간 X로 보인다 (재우기·연결 중은 그냥 이음줄이 없다)
    const relayFailed = cloudState === 'error';
    const rejected = relayFailed && !refused && relayErrorKind === 'rejected';
    const rateLimited = relayFailed && !refused && relayErrorKind === 'rate';
    const pairHere = pairing.active && (pairInConn || rejected);
    const repair = () => {
        setPairInConn(true);
        onStartPairing();
    };
    const showInfo = () => infoRef.current?.scrollIntoView({ behavior: 'smooth', block: 'center' });
    return (
        <div className="dc-view">
            {/* 연결 카드: 제목·상태·재우기 + 사바쨩 → 릴레이 서버 → 디스코드 흐름 + 그림 칸 (여러 카드를 하나로) */}
            <section className={clsx('dc-card dc-conn-card', { off: !agentRunning || refused })}>
                <div className="dc-conn-head">
                    <div className="dc-conn-title">
                        <h1>{t('discord.connection_title')}</h1>
                        <p className="muted">
                            {t(
                                connected
                                    ? agentRunning && !refused
                                        ? 'discord.conn_sub_connected'
                                        : 'discord.conn_sub_paused'
                                    : cloudState === 'connecting'
                                      ? 'discord.conn_sub_connecting'
                                      : 'discord.conn_sub_error',
                            )}
                        </p>
                    </div>
                    <div className="dc-conn-side">
                        <div className="dc-conn-state">
                            {connected &&
                                (heartbeatReject ? (
                                    <StatusChip tone="stop" label={t('discord.relay_failed')} />
                                ) : (
                                    <StatusChip tone="ok" label={t('discord.relay_connected')} />
                                ))}
                            {cloudState === 'connecting' && (
                                <StatusChip
                                    tone={reconnecting ? 'warn' : 'idle'}
                                    pulse
                                    label={t('discord.relay_connecting')}
                                />
                            )}
                            {cloudState === 'error' &&
                                (rateLimited ? (
                                    <StatusChip tone="warn" pulse label={t('discord.relay_recovering')} />
                                ) : (
                                    <StatusChip tone="stop" label={t('discord.relay_failed')} />
                                ))}
                            <button
                                type="button"
                                className="icon-btn"
                                aria-label={t('discord.conn_menu')}
                                data-tooltip={t('discord.conn_menu')}
                                onClick={(e) =>
                                    openMenu(e, [
                                        {
                                            id: 'pair',
                                            label: t('discord.pair_again'),
                                            icon: 'refresh',
                                            onSelect: repair,
                                        },
                                        {
                                            id: 'info',
                                            label: t('discord.connection_info_show'),
                                            icon: 'info',
                                            onSelect: showInfo,
                                        },
                                        'separator',
                                        {
                                            id: 'disconnect',
                                            label: t('discord.disconnect'),
                                            icon: 'cloudOff',
                                            danger: true,
                                            onSelect: showInfo,
                                        },
                                    ])
                                }
                            >
                                <Icon name="moreHorizontal" size="sm" />
                            </button>
                        </div>
                        {refused && agentRunning ? null : agentRunning ? (
                            <SleepAllButton nodes={nodes} onAsleepChange={setAsleep} />
                        ) : (
                            <button
                                type="button"
                                className="btn btn-primary"
                                onClick={onStartAgent}
                                disabled={!connected}
                            >
                                <Icon name="play" size="sm" /> {t('discord.agent_start')}
                            </button>
                        )}
                    </div>
                </div>

                {heartbeatReject && (
                    <div className="notice tone-stop">
                        <Icon name="alertTriangle" size="sm" />
                        <span className="dc-notice-text">
                            <strong>{t('discord.heartbeat_rejected_title')}</strong>
                            <span>{heartbeatRejectText(t, heartbeatReject)}</span>
                        </span>
                    </div>
                )}
                {/* 거절 이유는 아래 업데이트 안내가 말한다 */}
                {rejected && (
                    <div className="notice tone-stop dc-conn-rejected">
                        <Icon name="lock" size="sm" />
                        <span className="dc-notice-text">
                            <strong>{t('discord.relay_rejected_title')}</strong>
                            <span>{t('discord.relay_rejected_body')}</span>
                        </span>
                        <span className="dc-notice-actions">
                            <button type="button" className="btn btn-ghost btn-sm" onClick={onReload}>
                                {t('discord.reload')}
                            </button>
                            {!pairing.active && (
                                <button type="button" className="btn btn-primary btn-sm" onClick={repair}>
                                    <Icon name="plug" size="sm" /> {t('discord.repair_this_pc')}
                                </button>
                            )}
                        </span>
                    </div>
                )}
                {rateLimited && (
                    <div className="notice tone-warn">
                        <Icon name="clock" size="sm" />
                        <span className="dc-notice-text">
                            <strong>{t('discord.relay_rate_limited_title')}</strong>
                            <span>{t('discord.relay_rate_limited_body')}</span>
                        </span>
                        <button type="button" className="btn btn-ghost btn-sm" onClick={onReload}>
                            {t('discord.reload')}
                        </button>
                    </div>
                )}
                {cloudState === 'error' && !refused && !rejected && !rateLimited && (
                    <div className="notice tone-stop">
                        <Icon name="alertTriangle" size="sm" />
                        <span>{t('discord.relay_failed_body', { error: cloudError || '-' })}</span>
                        <button type="button" className="btn btn-ghost btn-sm" onClick={onReload}>
                            {t('discord.reload')}
                        </button>
                    </div>
                )}
                {pairHere && (
                    <PairingCard
                        pairing={pairing}
                        onCancel={() => {
                            setPairInConn(false);
                            onCancelPairing();
                        }}
                    />
                )}
                {agentReplaced && !agentRunning && (
                    <div className="notice tone-warn">
                        <Icon name="alertTriangle" size="sm" />
                        <span>{t('discord.agent_replaced')}</span>
                    </div>
                )}
                {agentMinVersion && (
                    <div className="notice tone-warn">
                        <Icon name="alertTriangle" size="sm" />
                        <span>{t('update_required.relay', { version: agentMinVersion })}</span>
                        <button
                            type="button"
                            className="btn btn-ghost btn-sm"
                            onClick={() => useUIStore.getState().openSettings('update')}
                        >
                            {t('update_required.check')}
                        </button>
                    </div>
                )}
                {agentRelayUrlInvalid && !agentRunning && (
                    <div className="notice tone-warn">
                        <Icon name="alertTriangle" size="sm" />
                        <span>{t('discord.invalid_relay_url')}</span>
                    </div>
                )}

                <div className="dc-conn-body">
                    {/* 명령이 흐르는 길: 이 PC의 사바쨩 → 릴레이 서버 → 디스코드. 끊긴 구간은 흐리게 */}
                    <ol className="dc-chain" aria-label={t('discord.agent_desc')}>
                        <li className={clsx('dc-chain-node', { on: flowing })}>
                            <img src="./icon.png" alt="" className="dc-chain-icon" />
                            <span className="dc-chain-name">{t('discord.chain_saba')}</span>
                            <span className="dc-chain-sub">
                                {t(
                                    `discord.agent_${botStatus === 'running' || botStatus === 'connecting' ? botStatus : 'stopped'}`,
                                )}
                            </span>
                        </li>
                        <ChainLink state={relayFailed ? 'fail' : flowing ? 'on' : 'off'} />
                        <li className={clsx('dc-chain-node', { on: connected, fail: relayFailed })}>
                            <img src="./server-chan.webp" alt="" className="dc-chain-icon" />
                            <span className="dc-chain-name">{t('discord.chain_relay')}</span>
                            <span className="dc-chain-sub">{relayHost(relayUrl)}</span>
                        </li>
                        <ChainLink state={connected ? 'on' : 'off'} />
                        <li className={clsx('dc-chain-node', { on: connected && nodeCount > 0 })}>
                            <span className="dc-chain-discord">
                                <Icon name="discordLogo" size={26} />
                            </span>
                            <span className="dc-chain-name">{t('discord.chain_discord')}</span>
                            <span className="dc-chain-sub">{t('discord.node_count', { count: nodeCount })}</span>
                        </li>
                    </ol>
                    <div className="dc-conn-art">
                        {/* 그림은 칸 안에서만: 다른 것이 침범하지 않는다. 명령 전달이 꺼지면 채도를 낮춘다 */}
                        <img src="./saba/relay.webp" alt="" draggable={false} decoding="async" />
                    </div>
                </div>
            </section>

            <section className="dc-card">
                <div className="dc-card-head">
                    <h2>{t('discord.add_server_title')}</h2>
                </div>
                <p className="dc-card-desc">{t('discord.add_server_desc')}</p>
                <div className="cb-toggle-row">
                    <span className="dc-list-text">
                        <span className="dc-list-name">{t('discord.new_guild_instances_title')}</span>
                        <span className="cb-hint">{t('discord.new_guild_instances_desc')}</span>
                    </span>
                    <SabaToggle
                        checked={newGuildInstances !== 'manual'}
                        aria-label={t('discord.new_guild_instances_title')}
                        onChange={(on) =>
                            useDiscordStore.getState().update({ discordNewGuildInstances: on ? 'auto' : 'manual' })
                        }
                    />
                </div>
                {pairing.active && !pairHere ? (
                    <PairingCard pairing={pairing} onCancel={onCancelPairing} />
                ) : pairHere ? null : (
                    <div className="dc-row-actions">
                        {onInviteBot && (
                            <button type="button" className="btn btn-ghost" onClick={onInviteBot}>
                                <Icon name="externalLink" size="sm" /> {t('discord.invite_bot')}
                            </button>
                        )}
                        <button type="button" className="btn btn-ghost" onClick={onStartPairing}>
                            <Icon name="plus" size="sm" /> {t('discord.pair_start')}
                        </button>
                    </div>
                )}
            </section>

            <PcNoticeCard />

            <section className="dc-card" ref={infoRef}>
                <div className="dc-card-head">
                    <h2>{t('discord.connection_info')}</h2>
                </div>
                <dl className="dc-facts">
                    <dt>{t('discord.host_id')}</dt>
                    <dd>
                        <code>{hostId}</code>
                        <CopyButton text={hostId} label={t('discord.copy')} />
                    </dd>
                    <dt>{t('discord.relay_url')}</dt>
                    <dd>
                        <code>{relayUrl}</code>
                    </dd>
                </dl>
                <div className="dc-row-actions dc-danger-zone">
                    <span className="muted">{t('discord.disconnect_desc')}</span>
                    <button
                        type="button"
                        className={clsx('btn', confirmDisconnect ? 'btn-danger' : 'btn-ghost btn-danger-text')}
                        onClick={() => {
                            if (confirmDisconnect) {
                                setConfirmDisconnect(false);
                                onDisconnect();
                            } else {
                                setConfirmDisconnect(true);
                            }
                        }}
                        onBlur={() => setConfirmDisconnect(false)}
                    >
                        <Icon name="cloudOff" size="sm" />{' '}
                        {confirmDisconnect ? t('discord.disconnect_confirm') : t('discord.disconnect')}
                    </button>
                </div>
            </section>
        </div>
    );
}

// PC 주인 DM 알림 묶음 (사용자 2026-11-08). 백업은 범위(끄기·실패만·모두), 나머지는 켜고 끈다
const DM_GROUPS: { label: string; kinds: DmNoticeKind[] }[] = [
    { label: 'dm_group_server', kinds: ['crash', 'externalStart', 'schedule'] },
    { label: 'dm_group_pc', kinds: ['update', 'disk'] },
    { label: 'dm_group_guild', kinds: ['chatbotUsage', 'moderation'] },
];
const DM_KEYS: Record<DmNoticeKind, string> = {
    crash: 'dm_crash',
    externalStart: 'dm_external_start',
    schedule: 'dm_schedule',
    update: 'dm_update',
    disk: 'dm_disk',
    chatbotUsage: 'dm_chatbot_usage',
    moderation: 'dm_moderation',
};

/** PC 알림 카드: PC 주인에게 가는 디스코드 DM을 종류별로 켜고 끈다 (데몬 봇 설정 `dmNotice`·`backupNotice`) */
function PcNoticeCard() {
    const { t } = useTranslation('gui');
    const dmNotice = useDiscordStore((s) => s.discordDmNotice);
    const backupNotice = useDiscordStore((s) => s.discordBackupNotice);
    const toggleRow = (kind: DmNoticeKind) => {
        const key = DM_KEYS[kind];
        return (
            <div className="cb-toggle-row" key={kind}>
                <span className="dc-list-text">
                    <span className="dc-list-name">{t(`discord.${key}_title`)}</span>
                    <span className="cb-hint">{t(`discord.${key}_desc`)}</span>
                    {kind === 'crash' && !dmNotice.crash && (
                        <span className="cb-hint dc-dm-warn">{t('discord.dm_crash_off_warn')}</span>
                    )}
                </span>
                <SabaToggle
                    checked={dmNotice[kind]}
                    aria-label={t(`discord.${key}_title`)}
                    onChange={(on) =>
                        useDiscordStore.getState().update({ discordDmNotice: { ...dmNotice, [kind]: on } })
                    }
                />
            </div>
        );
    };
    return (
        <section className="dc-card">
            <div className="dc-card-head">
                <h2>{t('discord.pc_notice_title')}</h2>
            </div>
            <p className="dc-card-desc">{t('discord.pc_notice_desc')}</p>
            {DM_GROUPS.map((group, i) => (
                <div className="dc-dm-group" key={group.label}>
                    <span className="dc-dm-group-label">{t(`discord.${group.label}`)}</span>
                    <div className="cb-toggles">
                        {group.kinds.map(toggleRow)}
                        {i === 0 && (
                            <div className="cb-toggle-row">
                                <span className="dc-list-text">
                                    <span className="dc-list-name">{t('discord.dm_backup_title')}</span>
                                    <span className="cb-hint">{t('discord.dm_backup_desc')}</span>
                                </span>
                                <Segmented
                                    ariaLabel={t('discord.dm_backup_title')}
                                    value={backupNotice}
                                    onChange={(v) =>
                                        useDiscordStore.getState().update({ discordBackupNotice: v as BackupNotice })
                                    }
                                    items={[
                                        { id: 'off', label: t('discord.dm_backup_off') },
                                        { id: 'failures', label: t('discord.dm_backup_failures') },
                                        { id: 'all', label: t('discord.dm_backup_all') },
                                    ]}
                                />
                            </div>
                        )}
                    </div>
                </div>
            ))}
        </section>
    );
}
