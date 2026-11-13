import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useServerStore } from '../../stores/useServerStore';
import type { Agreement, PlayerCount } from '../../types/daemon';
import { failed } from '../../utils/apiResult';
import { safeShowToast } from '../../utils/helpers';
import { controlOutcome } from '../../utils/serverControl';
import { gameNameOf, initials, isTransitional, STATUS_TONE, primaryAction } from '../../utils/serverView';
import { AgreementDialog } from '../AgreementList';
import { Icon } from '../Icon';
import { QuestionModal } from '../Modals/Modals';
import { remoteErrorText } from '../Modals/RemoteDaemonSection';
import { StatusChip } from '../ui/StatusChip';
import { startPoll } from '../../utils/poll';
import { reloadWindow } from './DaemonSwitcher';

const REFRESH_MS = 10_000;

/** GET /api/remote/servers 항목: 데몬 하나와 그 서버들 (이 PC는 hostId가 없다) */
interface DaemonServers {
    hostId?: string | null;
    label?: string;
    name?: string;
    ok?: boolean;
    error_code?: string;
    servers: { id: string; name: string; module: string; status: string; players?: PlayerCount }[];
}
type RemoteServer = DaemonServers['servers'][number];

/** 떠 있는 확인 창: 정지 확인, 저장 없이 정지, 약관 동의 (이 PC의 서버와 같은 창) */
type Ask = { hostId: string | null; pc: string; server: RemoteServer } & (
    | { kind: 'stop' }
    | { kind: 'agreements'; agreements: Agreement[] }
);

/**
 * 합쳐 보기 (G1c, 설정 `gui.daemonView = unified`): 서버 목록 아래에 고른 데몬이 아닌 데몬(이 PC 포함)의 서버를 데몬별로 보인다.
 * 서버는 id로 그 데몬에 켜고 끈다(`remoteInstanceAction`). 정지 확인·약관 동의는 이 PC의 서버와 같은 창을 거친다. 설정·콘솔 같은 나머지는 "이 데몬 열기"로 그 데몬을 고른 뒤 쓴다.
 * 고른 데몬의 목록(useServerStore)에 섞지 않는다: 동작은 그 데몬으로 보내고(`remoteInstanceAction`), 화면도 데몬별로 나뉜다.
 */
export function OtherDaemons({
    currentTarget,
    onSwitched = reloadWindow,
}: {
    currentTarget?: string | null;
    onSwitched?: () => void;
}) {
    const { t } = useTranslation('gui');
    const [daemons, setDaemons] = useState<DaemonServers[]>([]);
    const modules = useServerStore((s) => s.modules);
    const [busy, setBusy] = useState<string | null>(null);
    const [ask, setAsk] = useState<Ask | null>(null);

    const loading = useRef(false);

    // 닿지 않는 데몬은 답이 30초까지 늦다: 앞 조회가 끝나기 전에는 다시 묻지 않는다
    const load = useCallback(async () => {
        if (loading.current) return;
        loading.current = true;
        try {
            const res = await window.api.remoteServers().catch(() => null);
            if (Array.isArray(res?.daemons)) setDaemons(res.daemons);
        } finally {
            loading.current = false;
        }
    }, []);

    useEffect(() => {
        // 창이 숨어 있는 동안은 다른 데몬에 묻지 않는다
        return startPoll(load, REFRESH_MS);
    }, [load]);

    const run = async (target: Pick<Ask, 'hostId' | 'pc' | 'server'>, action: string) => {
        const { hostId, server } = target;
        setAsk(null);
        setBusy(server.id);
        try {
            const res = await window.api.remoteInstanceAction(hostId, server.id, action);
            const outcome = controlOutcome(res);
            if (outcome.kind === 'agreements' && outcome.agreements.length) {
                setAsk({ ...target, kind: 'agreements', agreements: outcome.agreements });
            } else if (failed(res)) {
                safeShowToast(res?.error || remoteErrorText(res, t), 'error', 5000);
            }
        } finally {
            setBusy(null);
            await load();
        }
    };

    const toggle = (hostId: string | null, pc: string, server: RemoteServer, action: string) =>
        action === 'stop' ? setAsk({ hostId, pc, server, kind: 'stop' }) : run({ hostId, pc, server }, action);

    /** 동의를 그 데몬에 기록하고 다시 켠다. 옛 데몬이라 기록하지 못하면 그 데몬을 열어서 하라고 알린다 */
    const accept = async (target: Ask, ids: string[]) => {
        setAsk(null);
        const res = await window.api.remoteInstanceAgreements(target.hostId, target.server.id, ids).catch(() => null);
        if (failed(res)) {
            safeShowToast(t('other_daemons.open_to_continue', { pc: target.pc }), 'error', 5000);
            return;
        }
        await run(target, 'start');
    };

    const open = async (hostId: string | null) => {
        const res = await window.api.daemonTargetSet(hostId).catch(() => null);
        if (res?.success) onSwitched();
    };

    const others = daemons.filter((d) => (d.hostId || null) !== (currentTarget || null));
    if (others.length === 0) return null;

    return (
        <div className="other-daemons">
            {others.map((d) => {
                const name = d.hostId ? d.label || d.name || d.hostId : t('daemon_switcher.local');
                return (
                    <section key={d.hostId || 'local'} className="other-daemon" aria-label={name}>
                        <header className="other-daemon-head">
                            <Icon name={d.hostId ? 'cloud' : 'desktop'} size="sm" />
                            <span className="other-daemon-name">{name}</span>
                            <button type="button" className="btn btn-ghost btn-sm" onClick={() => open(d.hostId || null)}>
                                {t('other_daemons.open')}
                            </button>
                        </header>
                        {!d.ok && <p className="muted pad">{remoteErrorText({ error_code: d.error_code }, t)}</p>}
                        {d.ok && d.servers.length === 0 && <p className="muted pad">{t('other_daemons.empty')}</p>}
                        {d.servers.map((s) => {
                            const action = busy === s.id ? 'busy' : primaryAction(s.status);
                            const label = t(`shell.status_${s.status}`, { defaultValue: s.status });
                            const moduleData = modules.find((m) => m.name === s.module);
                            const gameName = gameNameOf(t, s.module, moduleData);
                            return (
                                <div key={s.id} className="server-row other-daemon-row" data-status={s.status}>
                                    <div className="server-row-icon">
                                        {moduleData?.icon ? <img src={moduleData.icon} alt="" /> : <span>{initials(gameName)}</span>}
                                        <span
                                            className={clsx('server-row-dot', `tone-${STATUS_TONE[s.status]}`, {
                                                pulse: isTransitional(s.status),
                                            })}
                                            aria-hidden="true"
                                        />
                                    </div>
                                    <div className="server-row-text">
                                        <div className="server-row-name">{s.name}</div>
                                        <div className="server-row-sub">
                                            <span>{gameName}</span>
                                            <span className="sep">·</span>
                                            <StatusChip
                                                tone={STATUS_TONE[s.status]}
                                                pulse={isTransitional(s.status)}
                                                label={label}
                                                className="bare"
                                            />
                                        </div>
                                    </div>
                                    <button
                                        type="button"
                                        className={clsx('icon-btn', 'server-row-action', `action-${action}`)}
                                        disabled={action === 'busy'}
                                        onClick={() => toggle(d.hostId || null, name, s, action)}
                                        data-tooltip={action === 'stop' ? t('shell.stop') : action === 'start' ? t('shell.start') : label}
                                        aria-label={`${s.name} ${action === 'stop' ? t('shell.stop') : t('shell.start')}`}
                                    >
                                        {action === 'busy' ? (
                                            <span className="spinner" aria-hidden="true" />
                                        ) : (
                                            <Icon name={action === 'stop' ? 'stop' : 'play'} size="sm" />
                                        )}
                                    </button>
                                </div>
                            );
                        })}
                    </section>
                );
            })}
            {ask?.kind === 'stop' && (
                <QuestionModal
                    title={t('servers.stop_confirm_title')}
                    message={t('servers.stop_confirm_message', { name: ask.server.name })}
                    detail={[
                        t('other_daemons.stop_where', { pc: ask.pc }),
                        Number.isInteger(ask.server.players?.online) && (ask.server.players?.online as number) > 0
                            ? t('other_daemons.stop_players', { count: ask.server.players?.online })
                            : '',
                    ]
                        .filter(Boolean)
                        .join(' ')}
                    onConfirm={() => run(ask, 'stop')}
                    onCancel={() => setAsk(null)}
                />
            )}
            {ask?.kind === 'agreements' && (
                <AgreementDialog
                    title={t('agreements.start_title')}
                    message={t('agreements.start_message', { name: ask.server.name })}
                    module={ask.server.module}
                    agreements={ask.agreements}
                    note={t('other_daemons.agreement_where', { pc: ask.pc })}
                    onAccept={(ids) => accept(ask, ids)}
                    onCancel={() => setAsk(null)}
                />
            )}
        </div>
    );
}
