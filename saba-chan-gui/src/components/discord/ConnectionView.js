import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../../utils/helpers';
import { CliText } from '../CliLine';
import { Icon } from '../Icon';
import '../ui/SabaArt.css';
import { SabaSpinner } from '../ui/SabaUI';
import { StatusChip } from '../ui/StatusChip';

const failedRes = (res) => !res || res.success === false || Boolean(res.error_code) || Boolean(res.error);

/**
 * 재우기: 연결은 그대로 두고 모든 디스코드 서버의 잠자기(데몬 discord settings `sleeping`)를 켠다.
 * 모두 잠들어 있으면 깨우기로 바뀐다. 일부만 바뀐 채 실패하면 방금 하려던 쪽을 다시 누를 수 있게 둔다.
 */
function SleepAllButton({ nodes }) {
    const { t } = useTranslation('gui');
    const ids = (nodes || []).map((n) => n.guildId).join(',');
    const [asleep, setAsleep] = useState(null); // null = 아직 모름
    const [busy, setBusy] = useState(false);
    // 늦게 온 옛 길드 목록의 응답은 버린다
    const generation = useRef(0);
    // 일부만 잠든 상태에서 보여줄 쪽: 마지막으로 시도한 동작 (없으면 재우기)
    const lastAttempt = useRef(null);

    const load = useCallback(async () => {
        const gen = ++generation.current;
        const list = ids ? ids.split(',') : [];
        if (!list.length || !window.api?.discordSettingsGet) return setAsleep(false);
        const res = await Promise.all(list.map((g) => window.api.discordSettingsGet(g).catch(() => null)));
        if (gen !== generation.current) return;
        const sleepingCount = res.filter((r) => !failedRes(r) && r.sleeping === true).length;
        if (sleepingCount === list.length) setAsleep(true);
        else if (sleepingCount === 0) setAsleep(false);
        else setAsleep(lastAttempt.current === 'wake');
    }, [ids]);
    useEffect(() => {
        lastAttempt.current = null;
        load();
    }, [load]);

    const toggle = async () => {
        const sleeping = !asleep;
        lastAttempt.current = sleeping ? 'sleep' : 'wake';
        setBusy(true);
        const list = ids.split(',');
        const res = await Promise.all(
            list.map((g) => window.api.discordSettingsSet(g, { sleeping }).catch((e) => ({ error: e.message }))),
        );
        const bad = res.filter(failedRes).length;
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
        <button type="button" className="btn btn-secondary" onClick={toggle} disabled={disabled}>
            <Icon name="zzz" size="sm" /> {t('discord.agent_stop')}
        </button>
    );
}

function CopyButton({ text, label }) {
    const { t } = useTranslation('gui');
    const [copied, setCopied] = useState(false);
    return (
        <button
            type="button"
            className="btn btn-secondary btn-sm"
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

/** 페어링 진행 카드: 코드 → 디스코드에서 입력 → 자동 확인 */
export function PairingCard({ pairing, onCancel }) {
    const { t } = useTranslation('gui');
    const command = `/사바쨩 연결 코드:${pairing.code}`;

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
    if (pairing.status === 'expired' || pairing.status === 'error') {
        return (
            <div className="dc-card dc-pair tone-warn">
                <div className="dc-pair-result">
                    <Icon name="alertTriangle" size="lg" />
                    <strong>{t(pairing.status === 'expired' ? 'discord.pair_expired' : 'discord.pair_error')}</strong>
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
function ChainLink({ state }) {
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
function relayHost(url) {
    try {
        return new URL(url).host;
    } catch {
        return url || '-';
    }
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
    pairing,
    onStartPairing,
    onCancelPairing,
    onDisconnect,
    onStartAgent,
    nodes,
    onInviteBot,
}) {
    const { t } = useTranslation('gui');
    const [confirmDisconnect, setConfirmDisconnect] = useState(false);

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
                                <button type="button" className="btn btn-secondary btn-lg" onClick={onInviteBot}>
                                    <Icon name="externalLink" size="sm" /> {t('discord.invite_bot')}
                                </button>
                            )}
                            <button type="button" className="btn btn-primary btn-lg" onClick={onStartPairing}>
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
    // 릴레이에 닿지 못한 것은 "실패": 끊긴 자리를 빨간 X로 보인다 (재우기·연결 중은 그냥 이음줄이 없다)
    const relayFailed = cloudState === 'error';
    return (
        <div className="dc-view">
            {/* 연결 카드: 제목·상태·재우기 + 사바쨩 → 릴레이 서버 → 디스코드 흐름 + 그림 칸 (여러 카드를 하나로) */}
            <section className={clsx('dc-card dc-conn-card', { off: !agentRunning })}>
                <div className="dc-conn-head">
                    <div className="dc-conn-title">
                        <h1>{t('discord.connection_title')}</h1>
                        <p className="muted">
                            {t(
                                connected
                                    ? 'discord.conn_sub_connected'
                                    : cloudState === 'connecting'
                                      ? 'discord.conn_sub_connecting'
                                      : 'discord.conn_sub_error',
                            )}
                        </p>
                    </div>
                    <div className="dc-conn-side">
                        <div className="dc-conn-state">
                            {connected && <StatusChip tone="ok" label={t('discord.relay_connected')} />}
                            {cloudState === 'connecting' && (
                                <StatusChip tone="warn" pulse label={t('discord.relay_connecting')} />
                            )}
                            {cloudState === 'error' && <StatusChip tone="stop" label={t('discord.relay_failed')} />}
                        </div>
                        {agentRunning ? (
                            <SleepAllButton nodes={nodes} />
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

                {cloudState === 'error' && (
                    <div className="notice tone-stop">
                        <Icon name="alertTriangle" size="sm" />
                        <span>{t('discord.relay_failed_body', { error: cloudError || '-' })}</span>
                    </div>
                )}
                {agentReplaced && !agentRunning && (
                    <div className="notice tone-warn">
                        <Icon name="alertTriangle" size="sm" />
                        <span>{t('discord.agent_replaced')}</span>
                    </div>
                )}

                <div className="dc-conn-body">
                    {/* 명령이 흐르는 길: 이 PC의 사바쨩 → 릴레이 서버 → 디스코드. 끊긴 구간은 흐리게 */}
                    <ol className="dc-chain" aria-label={t('discord.agent_desc')}>
                        <li className={clsx('dc-chain-node', { on: agentRunning && connected })}>
                            <img src="./icon.png" alt="" className="dc-chain-icon" />
                            <span className="dc-chain-name">{t('discord.chain_saba')}</span>
                            <span className="dc-chain-sub">
                                {t(
                                    `discord.agent_${botStatus === 'running' || botStatus === 'connecting' ? botStatus : 'stopped'}`,
                                )}
                            </span>
                        </li>
                        <ChainLink state={relayFailed ? 'fail' : agentRunning && connected ? 'on' : 'off'} />
                        <li className={clsx('dc-chain-node', { on: connected, fail: relayFailed })}>
                            <Icon name={relayFailed ? 'cloudOff' : 'cloud'} size={26} />
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
                {pairing.active ? (
                    <PairingCard pairing={pairing} onCancel={onCancelPairing} />
                ) : (
                    <div className="dc-row-actions">
                        {onInviteBot && (
                            <button type="button" className="btn btn-ghost" onClick={onInviteBot}>
                                <Icon name="externalLink" size="sm" /> {t('discord.invite_bot')}
                            </button>
                        )}
                        <button type="button" className="btn btn-secondary" onClick={onStartPairing}>
                            <Icon name="plus" size="sm" /> {t('discord.pair_start')}
                        </button>
                    </div>
                )}
            </section>

            <section className="dc-card">
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
                        className={clsx('btn', confirmDisconnect ? 'btn-danger' : 'btn-danger-text')}
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
