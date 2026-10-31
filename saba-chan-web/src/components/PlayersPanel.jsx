import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from './icons';
import { Button, Empty, errorText, failed, Loading, Section } from './ui';

/**
 * 지금 접속한 사람. 열 때와 새로고침을 누를 때만 묻는다
 * (게임에 사람 수를 주기적으로 묻지 않는다: 들어옴·나감은 콘솔 줄로 센다)
 */
export function PlayersPanel({ api, server, running }) {
    const { t } = useTranslation();
    const [state, setState] = useState(null);
    const [busy, setBusy] = useState(false);

    const load = useCallback(async () => {
        if (!running) return setState({ stopped: true });
        setBusy(true);
        const res = await api.instancePlayers(server.id);
        setBusy(false);
        if (res?.error_code === 'not_supported') return setState({ unsupported: true });
        if (res?.error_code === 'server_not_running') return setState({ stopped: true });
        if (failed(res) || !Array.isArray(res?.players)) return setState({ error: errorText(t, res) });
        setState({ online: res.online ?? res.players.length, max: res.max, players: res.players });
    }, [api, server.id, running, t]);

    useEffect(() => {
        load();
    }, [load]);

    const count = state?.players ? (state.max ? `${state.online} / ${state.max}` : String(state.online)) : null;

    return (
        <Section
            title={count ? t('players.title_count', { count }) : t('detail.players')}
            action={
                running && (
                    <Button busy={busy} onClick={load}>
                        <Icon name="refresh" size={16} /> {t('players.refresh')}
                    </Button>
                )
            }
        >
            {state === null ? (
                <Loading />
            ) : state.stopped ? (
                <Empty>{t('players.stopped')}</Empty>
            ) : state.unsupported ? (
                <Empty>{t('players.unsupported')}</Empty>
            ) : state.error ? (
                <p className="banner banner-error">{state.error}</p>
            ) : state.players.length === 0 ? (
                <Empty>{t('players.none')}</Empty>
            ) : (
                <ul className="player-list">
                    {state.players.map((p, i) => (
                        <li key={`${p.name}-${i}`} className="player">
                            <span className="player-avatar" aria-hidden="true">
                                {Array.from(String(p.name || '?'))[0].toUpperCase()}
                            </span>
                            {p.name}
                        </li>
                    ))}
                </ul>
            )}
        </Section>
    );
}
