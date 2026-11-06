import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from './icons';
import { Button, Empty, errorText, failed, Loading, Section } from './ui';
import type { Server, WebApi } from '../types';

/** 접속한 사람 한 명 (모듈 players 함수의 답) */
interface Player {
    name?: string;
}

/** 화면 상태: 정지·미지원·오류 중 하나이거나 접속자 목록 */
interface PlayersState {
    stopped?: boolean;
    unsupported?: boolean;
    error?: string;
    online?: number;
    max?: number;
    players?: Player[];
}

/**
 * 지금 접속한 사람. 열 때와 새로고침을 누를 때만 묻는다
 * (게임에 사람 수를 주기적으로 묻지 않는다: 들어옴·나감은 콘솔 줄로 센다)
 */
export function PlayersPanel({ api, server, running }: { api: WebApi; server: Pick<Server, 'id'>; running: boolean }) {
    const { t } = useTranslation();
    const [state, setState] = useState<PlayersState | null>(null);
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
            title={count ? t('players.title_count', { count: count as unknown as number }) : t('detail.players')}
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
            ) : (state.players ?? []).length === 0 ? (
                <Empty>{t('players.none')}</Empty>
            ) : (
                <ul className="player-list">
                    {(state.players ?? []).map((p, i) => (
                        // biome-ignore lint/suspicious/noArrayIndexKey: 플레이어 이름이 겹칠 수 있어 순번을 붙인다
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
