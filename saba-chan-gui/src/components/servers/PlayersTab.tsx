import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { type Server, useServerStore } from '../../stores/useServerStore';
import type { CommandInput, ModuleInfo, PlayerList } from '../../types/daemon';
import { commandErrorText } from '../../utils/commandReply';
import { commandArgs } from '../../utils/commandSyntax';
import { safeShowToast } from '../../utils/helpers';
import { commandInputLabel } from '../../utils/serverView';
import { Avatar } from '../discord/Avatar';
import { Icon } from '../Icon';
import { SabaToggle } from '../ui/SabaUI';
import { Segmented } from '../ui/StatusChip';

/** 이 모듈이 플레이어 목록(`[[lists]]`)을 선언했고 데몬이 그 목록을 읽어 줄 수 있는가 */
export const playerListsAvailable = (moduleData?: ModuleInfo | null) =>
    !!moduleData?.lists?.length && typeof window.api?.instanceLists === 'function';

/**
 * 플레이어 탭: 모듈이 선언한 목록(화이트리스트·관리자·밴 등)의 지금 명단.
 * 명단은 데몬이 읽어 주고(GET …/lists), 넣기·빼기는 모듈이 그 목록에 붙인 명령을 그대로 보낸다.
 */
export function PlayersTab({ server, moduleData }: { server: Server; moduleData?: ModuleInfo | null }) {
    const { t, i18n } = useTranslation('gui');
    const [lists, setLists] = useState<PlayerList[] | null>(null);
    const [error, setError] = useState('');
    const [activeId, setActiveId] = useState('');
    const [values, setValues] = useState<Record<string, string>>({});
    const [busy, setBusy] = useState(false);
    const [online, setOnline] = useState<Set<string>>(new Set());
    const alive = useRef(true);
    useEffect(() => {
        alive.current = true;
        return () => {
            alive.current = false;
        };
    }, []);

    const load = useCallback(async () => {
        const res = await window.api.instanceLists(server.id);
        if (!alive.current) return;
        if (Array.isArray(res?.lists)) {
            setLists(res.lists);
            setError('');
        } else {
            setError(res?.error || res?.message || res?.error_code || '-');
        }
    }, [server.id]);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 서버가 켜지고 꺼지면 넣기·빼기 가능 여부가 달라진다
    useEffect(() => {
        load();
    }, [load, server.status]);
    // 지금 접속한 사람: 탭을 열 때 한 번만 묻는다 (주기적으로 묻지 않는다)
    useEffect(() => {
        if (server.status !== 'running' || typeof window.api?.instancePlayers !== 'function') {
            setOnline(new Set());
            return;
        }
        let current = true;
        window.api.instancePlayers(server.id).then((res) => {
            const names = Array.isArray(res?.players) ? res.players.map((p: { name?: string }) => String(p?.name || '').toLowerCase()) : [];
            if (current) setOnline(new Set(names));
        });
        return () => {
            current = false;
        };
    }, [server.id, server.status]);

    if (!lists) {
        return (
            <section className="dc-card" aria-label={t('players.title')}>
                <div className="dc-card-head">
                    <h2>{t('players.title')}</h2>
                </div>
                {error && (
                    <div className="notice tone-warn" role="alert">
                        {t('players.load_failed', { error })}
                    </div>
                )}
            </section>
        );
    }
    const active = lists.find((l) => l.id === activeId) || lists[0];
    if (!active) return null;

    const commands = moduleData?.commands?.fields || [];
    const inputsOf = (name: string): CommandInput[] => commands.find((c) => c.name === name)?.inputs || [];
    const addInputs = inputsOf(active.add);
    const [nameInput, ...extraInputs] = addInputs;
    const setting = active.setting ? moduleData?.settings?.fields.find((f) => f.name === active.setting) : undefined;
    const enabled = setting ? (server.module_settings?.[setting.name] ?? setting.default) === true : false;

    const send = async (command: string, args: Record<string, unknown>) => {
        setBusy(true);
        try {
            const res = await window.api.executeCommand(server.id, { command, args });
            if (!res || res.error || res.success === false) {
                safeShowToast(t('players.failed', { error: commandErrorText(res, t) ?? res?.error ?? '-' }), 'error', 5000);
                return false;
            }
            await load();
            // 서버가 답한 뒤에 파일을 고치는 게임도 있어 한 번 더 읽는다
            setTimeout(() => alive.current && load(), 1500);
            return true;
        } finally {
            if (alive.current) setBusy(false);
        }
    };
    const add = async () => {
        if (!nameInput || !String(values[nameInput.name] || '').trim()) return;
        if (await send(active.add, commandArgs(addInputs, values))) setValues({});
    };
    const remove = (name: string) => {
        const first = inputsOf(active.remove)[0];
        if (first) send(active.remove, { [first.name]: name });
    };
    const toggle = async (next: boolean) => {
        if (!setting) return;
        setBusy(true);
        try {
            const res = await window.api.instanceUpdateSettings(server.id, { [setting.name]: next });
            if (!res || res.success === false || res.error) {
                safeShowToast(t('players.setting_failed', { error: res?.error || '-' }), 'error', 5000);
                return;
            }
            useServerStore
                .getState()
                .setServers((prev) =>
                    prev.map((s) => (s.id === server.id ? { ...s, module_settings: { ...s.module_settings, [setting.name]: next } } : s)),
                );
            if (server.status === 'running' && setting.restart_required) safeShowToast(t('players.restart_to_apply'), 'info', 4000);
        } finally {
            if (alive.current) setBusy(false);
        }
    };

    const day = (iso?: string) => {
        const at = iso ? new Date(iso) : null;
        if (!at || Number.isNaN(at.getTime())) return '';
        return new Intl.DateTimeFormat(i18n.language, { month: 'long', day: 'numeric' }).format(at);
    };
    const subOf = (entry: PlayerList['entries'][number]) => {
        const date = day(entry.added);
        if (entry.note || active.danger) return [entry.note || t('players.no_reason'), date].filter(Boolean).join(' · ');
        return date ? t('players.added_on', { date }) : '';
    };
    const useLabel = t('players.use_list', { list: active.label });

    return (
        <section className="dc-card players-card" aria-label={t('players.title')}>
            <div className="dc-card-head">
                <h2>{t('players.title')}</h2>
                {setting && typeof window.api?.instanceUpdateSettings === 'function' && (
                    <span className="players-use">
                        <SabaToggle checked={enabled} disabled={busy} aria-label={useLabel} onChange={toggle} />
                        <span className="muted">{useLabel}</span>
                    </span>
                )}
            </div>
            <p className="dc-card-desc">{t(active.editable ? 'players.desc' : 'players.desc_offline')}</p>
            <Segmented
                items={lists.map((l) => ({ id: l.id, label: `${l.label} ${l.entries.length}` }))}
                value={active.id}
                onChange={(id) => {
                    setActiveId(id);
                    setValues({});
                }}
                ariaLabel={t('players.kinds')}
            />
            {nameInput && (
                <form
                    className="controllers-add players-add"
                    onSubmit={(e) => {
                        e.preventDefault();
                        add();
                    }}
                >
                    <input
                        type="text"
                        className="input"
                        value={values[nameInput.name] || ''}
                        maxLength={64}
                        disabled={!active.editable || busy}
                        placeholder={t('players.name_placeholder')}
                        aria-label={t('players.name_placeholder')}
                        onChange={(e) => setValues((v) => ({ ...v, [nameInput.name]: e.target.value }))}
                    />
                    {extraInputs.map((input) => {
                        const label = commandInputLabel(t, server.module, active.add, input);
                        const shown = input.required ? label : t('players.optional', { label });
                        return (
                            <input
                                key={input.name}
                                type="text"
                                className="input players-extra"
                                value={values[input.name] || ''}
                                maxLength={200}
                                disabled={!active.editable || busy}
                                placeholder={shown}
                                aria-label={shown}
                                onChange={(e) => setValues((v) => ({ ...v, [input.name]: e.target.value }))}
                            />
                        );
                    })}
                    <button
                        type="submit"
                        className={active.danger ? 'btn btn-danger btn-sm' : 'btn btn-primary btn-sm'}
                        disabled={!active.editable || busy}
                    >
                        {active.danger ? (
                            active.label
                        ) : (
                            <>
                                <Icon name="plus" size="sm" />
                                {t('players.add')}
                            </>
                        )}
                    </button>
                </form>
            )}
            {active.entries.length === 0 ? (
                <p className="muted">{t('players.empty')}</p>
            ) : (
                <ul className="dc-list">
                    {active.entries.map((entry) => {
                        const sub = subOf(entry);
                        return (
                            <li key={entry.name} className="dc-list-row">
                                <Avatar name={entry.name} size={28} />
                                <span className="dc-list-text">
                                    <span className="dc-list-name">
                                        {entry.name}
                                        {online.has(entry.name.toLowerCase()) && (
                                            <span className="players-online" role="img" aria-label={t('players.online')} data-tooltip={t('players.online')} />
                                        )}
                                    </span>
                                    {sub && <span className="dc-list-sub">{sub}</span>}
                                </span>
                                <button
                                    type="button"
                                    className="icon-btn"
                                    disabled={!active.editable || busy}
                                    aria-label={t('players.remove_named', { name: entry.name })}
                                    data-tooltip={t('players.remove_named', { name: entry.name })}
                                    onClick={() => remove(entry.name)}
                                >
                                    <Icon name="close" size="sm" />
                                </button>
                            </li>
                        );
                    })}
                </ul>
            )}
        </section>
    );
}
