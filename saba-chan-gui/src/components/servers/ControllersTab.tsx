import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useDiscordStore } from '../../stores/useDiscordStore';
import { type Server, useServerStore } from '../../stores/useServerStore';
import type { Controller } from '../../types/daemon';
import { safeShowToast } from '../../utils/helpers';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Avatar } from '../discord/Avatar';
import type { MembersByGuild } from '../discord/types';
import { Icon } from '../Icon';
import { addController, memberChoices, removeController } from './controllers';

/** 디스코드 멤버 목록: 디스코드 화면이 받아 둔 것, 없으면 이은 서버마다 한 번 받는다 (5곳까지) */
function useMemberChoices() {
    const cached = useDiscordStore((s) => s.cloudMembers);
    const nodes = useDiscordStore((s) => s.cloudNodes);
    const relayUrl = useDiscordStore((s) => s.discordCloudRelayUrl);
    const [loaded, setLoaded] = useState<MembersByGuild>({});
    const missing = nodes
        .map((n) => n.guildId)
        .filter((g) => !(cached[g]?.length > 0))
        .slice(0, 5);
    const missingKey = missing.join(',');
    // biome-ignore lint/correctness/useExhaustiveDependencies: 빠진 길드 목록이 바뀔 때만 받는다
    useEffect(() => {
        if (!missingKey || typeof window.api?.relayListNodeMembers !== 'function') return;
        let alive = true;
        for (const guildId of missing) {
            window.api.relayListNodeMembers(guildId, relayUrl).then((data) => {
                const members = Array.isArray(data) ? data : data?.members;
                if (alive && Array.isArray(members)) setLoaded((prev) => ({ ...prev, [guildId]: members }));
            });
        }
        return () => {
            alive = false;
        };
    }, [missingKey, relayUrl]);
    return useMemo(() => memberChoices({ ...loaded, ...cached }), [loaded, cached]);
}

/** 인스턴스 "조작 가능" 탭: 시작·정지·재시작·백업 만들기를 할 수 있는 사람 명단 (사용자 지정 2026-11-06) */
export function ControllersTab({ server }: { server: Server }) {
    const { t } = useTranslation('gui');
    const [list, setList] = useState<Controller[]>(server.controllers || []);
    const [name, setName] = useState('');
    const [discordId, setDiscordId] = useState('');
    const [busy, setBusy] = useState(false);
    const members = useMemberChoices();
    // biome-ignore lint/correctness/useExhaustiveDependencies: 다른 서버를 열거나 데몬 값이 바뀌었을 때만
    useEffect(() => {
        setList(server.controllers || []);
    }, [server.id, JSON.stringify(server.controllers || [])]);

    const save = async (next: Controller[]) => {
        setBusy(true);
        try {
            const res = await window.api.instanceUpdateSettings(server.id, { controllers: next });
            if (!res || res.success === false || res.error) {
                const gateOn = res?.fields?.some?.((f: { field?: string }) => f.field === 'controllers_only');
                safeShowToast(
                    gateOn ? t('controllers.err_gate_on') : t('controllers.save_failed', { error: res?.error || '-' }),
                    'error',
                    5000,
                );
                return false;
            }
            setList(next);
            useServerStore
                .getState()
                .setServers((prev) => prev.map((s) => (s.id === server.id ? { ...s, controllers: next } : s)));
            return true;
        } finally {
            setBusy(false);
        }
    };

    const add = async () => {
        const out = addController(list, name, discordId);
        if ('error' in out) {
            safeShowToast(t(`controllers.err_${out.error}`), 'warning', 4000);
            return;
        }
        if (await save(out.list)) {
            setName('');
            setDiscordId('');
        }
    };

    const member = (id?: string) => members.find((m) => m.id === id);

    return (
        <div className="protocol-mode-section controllers-section">
            <div className="protocol-mode-header">
                <span className="protocol-mode-title">
                    <Icon name="user" size="sm" /> {t('shell.tab_controllers')}
                </span>
            </div>
            <p className="protocol-mode-description">{t('controllers.desc')} {t('controllers.all_guilds')}</p>
            {list.length === 0 ? (
                <p className="muted controllers-empty">{t('controllers.empty')}</p>
            ) : (
                <ul className="controllers-list">
                    {list.map((c, i) => {
                        const m = member(c.discord_id);
                        return (
                            <li key={c.name} className="restart-row">
                                <span className="controllers-name">
                                    <strong>{c.name}</strong>
                                </span>
                                <span className="controllers-discord">
                                    {c.discord_id ? (
                                        <>
                                            <Avatar name={m?.label || c.discord_id} url={m?.avatarUrl} size={20} />
                                            {m?.label || c.discord_id}
                                        </>
                                    ) : (
                                        <span className="muted">{t('controllers.not_linked')}</span>
                                    )}
                                </span>
                                <button
                                    type="button"
                                    className="btn btn-ghost btn-sm"
                                    disabled={busy}
                                    aria-label={t('controllers.remove_named', { name: c.name })}
                                    onClick={() => save(removeController(list, i))}
                                >
                                    {t('controllers.remove')}
                                </button>
                            </li>
                        );
                    })}
                </ul>
            )}
            <form
                className="controllers-add"
                onSubmit={(e) => {
                    e.preventDefault();
                    add();
                }}
            >
                <input
                    type="text"
                    className="input"
                    value={name}
                    maxLength={64}
                    placeholder={t('controllers.name_placeholder')}
                    aria-label={t('controllers.name_placeholder')}
                    onChange={(e) => setName(e.target.value)}
                />
                <CustomDropdown
                    className="controllers-member"
                    value={discordId}
                    ariaLabel={t('controllers.member_none')}
                    onChange={(v) => setDiscordId(String(v))}
                    options={[
                        { value: '', label: t('controllers.member_none') },
                        ...members.map((m) => ({ value: m.id, label: m.label })),
                    ]}
                />
                <button type="submit" className="btn btn-primary btn-sm" disabled={busy || !name.trim()}>
                    {t('controllers.add')}
                </button>
            </form>
            {members.length === 0 && <p className="protocol-mode-description">{t('controllers.members_hint')}</p>}
            <p className="protocol-mode-description">{t('controllers.note')}</p>
        </div>
    );
}
