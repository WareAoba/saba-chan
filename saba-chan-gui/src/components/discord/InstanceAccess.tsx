import type { TFunction } from 'i18next';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { type Server, useServerStore } from '../../stores/useServerStore';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';
import { SabaToggle } from '../ui/SabaUI';
import { Segmented } from '../ui/StatusChip';

/**
 * 디스코드 › 서버 › 인스턴스 메뉴의 사용 모드 (데몬 인스턴스 access·kick_players·controllers_only, 사용자 지정 2026-11-08).
 * - 서버 공용: 길드 레벨 권한을 따른다. 자동 강퇴할 닉네임을 적는다
 * - 전용 멤버: 길드 레벨과 따로 논다. 전용 멤버 명단(ControllersTab)에 든 사람만 모든 조작·명령을 한다.
 *   명단에 없는 사람 자동 강퇴를 켜고 끈다
 * 모드와 명단은 인스턴스 하나에 붙어 이은 모든 디스코드 서버에 같이 적용된다.
 */
export type Access = 'shared' | 'members';

export const accessOf = (server: Server): Access => (server.access === 'members' ? 'members' : 'shared');

/** 인스턴스 설정 일부를 저장하고 화면의 서버 목록에도 넣는다. 실패하면 알리고 false */
async function patch(server: Server, values: Partial<Server>, failKey: string, t: TFunction) {
    const res = await window.api.instanceUpdateSettings(server.id, values);
    if (!res || res.success === false || res.error) {
        safeShowToast(t(failKey, { error: res?.error || '-' }), 'error', 5000);
        return false;
    }
    useServerStore.getState().setServers((prev) => prev.map((s) => (s.id === server.id ? { ...s, ...values } : s)));
    return true;
}

/** 모드 고르기: 바로 저장한다 */
export function AccessMode({ server }: { server: Server }) {
    const { t } = useTranslation('gui');
    const [mode, setMode] = useState<Access>(accessOf(server));
    const [busy, setBusy] = useState(false);
    useEffect(() => setMode(accessOf(server)), [server]);
    const change = async (next: string) => {
        if (next === mode || busy) return;
        setBusy(true);
        const ok = await patch(server, { access: next as Access }, 'access.save_failed', t);
        if (ok) setMode(next as Access);
        setBusy(false);
    };
    return (
        <div className="protocol-mode-section access-section">
            <div className="protocol-mode-header">
                <span className="protocol-mode-title">
                    <Icon name="key" size="sm" /> {t('access.title')}
                </span>
            </div>
            <Segmented
                items={[
                    { id: 'shared', label: t('access.shared') },
                    { id: 'members', label: t('access.members') },
                ]}
                value={mode}
                onChange={change}
                ariaLabel={t('access.title')}
            />
            <p className="protocol-mode-description">
                {t(mode === 'members' ? 'access.members_desc' : 'access.shared_desc')} {t('controllers.all_guilds')}
            </p>
        </div>
    );
}

/** 서버 공용의 자동 강퇴 명단: 적은 닉네임이 들어오면 내보낸다 */
export function KickList({ server }: { server: Server }) {
    const { t } = useTranslation('gui');
    const [list, setList] = useState<string[]>(server.kick_players || []);
    const [name, setName] = useState('');
    const [busy, setBusy] = useState(false);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 다른 서버를 열거나 데몬 값이 바뀌었을 때만
    useEffect(() => setList(server.kick_players || []), [server.id, JSON.stringify(server.kick_players || [])]);
    const save = async (next: string[]) => {
        setBusy(true);
        const ok = await patch(server, { kick_players: next }, 'kick.save_failed', t);
        if (ok) setList(next);
        setBusy(false);
        return ok;
    };
    const add = async () => {
        const trimmed = name.trim();
        if (!trimmed) return;
        if (list.some((n) => n.toLowerCase() === trimmed.toLowerCase())) {
            safeShowToast(t('controllers.err_duplicate'), 'warning', 4000);
            return;
        }
        if (await save([...list, trimmed])) setName('');
    };
    return (
        <div className="protocol-mode-section kick-section">
            <div className="protocol-mode-header">
                <span className="protocol-mode-title">
                    <Icon name="x" size="sm" /> {t('kick.title')}
                </span>
            </div>
            <p className="protocol-mode-description">{t('kick.desc')}</p>
            {!server.controllers_only_supported && <p className="protocol-mode-description">{t('kick.unsupported')}</p>}
            {list.length === 0 ? (
                <p className="muted controllers-empty">{t('kick.empty')}</p>
            ) : (
                <ul className="controllers-list">
                    {list.map((n) => (
                        <li key={n} className="restart-row">
                            <span className="controllers-name">
                                <strong>{n}</strong>
                            </span>
                            <button
                                type="button"
                                className="btn btn-ghost btn-sm"
                                disabled={busy}
                                aria-label={t('controllers.remove_named', { name: n })}
                                onClick={() => save(list.filter((x) => x !== n))}
                            >
                                {t('controllers.remove')}
                            </button>
                        </li>
                    ))}
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
                    aria-label={t('kick.placeholder')}
                    onChange={(e) => setName(e.target.value)}
                />
                <button type="submit" className="btn btn-primary btn-sm" disabled={busy || !name.trim()}>
                    {t('kick.add')}
                </button>
            </form>
            <p className="protocol-mode-description">{t('controllers_only.hint')}</p>
        </div>
    );
}

/** 전용 멤버의 자동 강퇴 스위치: 명단에 없는 닉네임이 들어오면 내보낸다. 명단이 비면 켤 수 없다 */
export function MembersKick({ server }: { server: Server }) {
    const { t } = useTranslation('gui');
    const [on, setOn] = useState(!!server.controllers_only);
    const [busy, setBusy] = useState(false);
    useEffect(() => setOn(!!server.controllers_only), [server.controllers_only]);
    const empty = (server.controllers || []).length === 0;
    if (!server.controllers_only_supported) {
        return <p className="protocol-mode-description">{t('kick.unsupported')}</p>;
    }
    return (
        <div className="protocol-mode-section controllers-only-section">
            <div className="restart-row">
                <span>
                    <strong>{t('controllers_only.label')}</strong>
                    <small>{empty ? t('controllers_only.empty') : t('controllers_only.hint')}</small>
                </span>
                <SabaToggle
                    checked={on}
                    disabled={busy || (empty && !on)}
                    onChange={async (checked) => {
                        setBusy(true);
                        if (await patch(server, { controllers_only: checked }, 'controllers.save_failed', t))
                            setOn(checked);
                        setBusy(false);
                    }}
                />
            </div>
        </div>
    );
}
