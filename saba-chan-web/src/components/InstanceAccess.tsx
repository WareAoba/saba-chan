import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp, useData } from '../store';
import type { Server, WebApi } from '../types';
import { Icon } from './icons';
import { Button, errorText, failed, Segments, Sheet } from './ui';

/**
 * 디스코드 › 서버 › 인스턴스 메뉴의 사용 모드 (GUI InstanceAccess 와 같다, 사용자 지정 2026-11-08).
 * 서버 공용: 길드 레벨 권한 + 자동 강퇴 닉네임(작은 단추 → 시트). 전용 멤버: 전용 멤버 명단 + 명단 밖 자동 강퇴 스위치.
 * 모드와 명단은 인스턴스에 붙어 이은 모든 디스코드 서버에 같이 적용된다.
 */
export type Access = 'shared' | 'members';

export const accessOf = (server: Server | undefined): Access => (server?.access === 'members' ? 'members' : 'shared');

/** 인스턴스 설정 일부를 저장하고 화면의 서버 목록에도 넣는다 */
function usePatch(api: WebApi, server: Server) {
    const { t } = useTranslation();
    const toast = useApp((s) => s.toast);
    return async (values: Partial<Server>) => {
        const res = await api.instanceUpdateSettings(server.id, values);
        if (failed(res)) {
            toast(errorText(t, res), 'error');
            return false;
        }
        const { servers, setServers } = useData.getState();
        setServers(servers.map((s) => (s.id === server.id ? { ...s, ...values } : s)));
        return true;
    };
}

/** 모드 줄: 제목·고르기, 오른쪽에 그 모드의 작은 단추(children), 아래 한 줄 설명. 바로 저장한다 */
export function AccessBar({ api, server, children }: { api: WebApi; server: Server; children?: React.ReactNode }) {
    const { t: tg } = useTranslation('gui');
    const patch = usePatch(api, server);
    const [mode, setMode] = useState<Access>(accessOf(server));
    const [busy, setBusy] = useState(false);
    useEffect(() => setMode(accessOf(server)), [server]);
    const change = async (next: string) => {
        if (next === mode || busy) return;
        setBusy(true);
        if (await patch({ access: next as Access })) setMode(next as Access);
        setBusy(false);
    };
    return (
        <div className="mode-bar-wrap">
            <div className="mode-bar">
                <span className="mode-bar-title">
                    <Icon name="key" size={16} /> {tg('access.title')}
                </span>
                <Segments
                    value={mode}
                    onChange={change}
                    ariaLabel={tg('access.title')}
                    options={[
                        { value: 'shared', label: tg('access.shared') },
                        { value: 'members', label: tg('access.members') },
                    ]}
                />
                {children && <span className="mode-bar-end">{children}</span>}
            </div>
            <small className="hint">
                {tg(mode === 'members' ? 'access.members_desc' : 'access.shared_desc')} {tg('access.applies_all')}
            </small>
        </div>
    );
}

/** 서버 공용의 자동 강퇴: 작은 단추(적은 수)를 누르면 시트에서 닉네임을 넣고 뺀다 */
export function KickButton({ api, server }: { api: WebApi; server: Server }) {
    const { t: tg } = useTranslation('gui');
    const toast = useApp((s) => s.toast);
    const patch = usePatch(api, server);
    const [open, setOpen] = useState(false);
    const [list, setList] = useState<string[]>(server.kick_players || []);
    const [name, setName] = useState('');
    const [busy, setBusy] = useState(false);
    const savedKey = JSON.stringify(server.kick_players || []);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 데몬 값이 바뀌었을 때만 따라간다
    useEffect(() => setList(server.kick_players || []), [savedKey]);
    const save = async (next: string[]) => {
        setBusy(true);
        const ok = await patch({ kick_players: next });
        if (ok) setList(next);
        setBusy(false);
        return ok;
    };
    const add = async () => {
        const trimmed = name.trim();
        if (!trimmed) return;
        if (list.some((n) => n.toLowerCase() === trimmed.toLowerCase())) return toast(tg('controllers.err_duplicate'), 'error');
        if (await save([...list, trimmed])) setName('');
    };
    return (
        <>
            <Button onClick={() => setOpen(true)} aria-haspopup="dialog">
                <Icon name="x" size={16} /> {tg('kick.title')}
                {list.length > 0 && <span className="count-pill">{list.length}</span>}
            </Button>
            {open && (
                <Sheet title={tg('kick.title')} onClose={() => setOpen(false)}>
                    <p className="hint">{tg('kick.desc')}</p>
                    {!server.controllers_only_supported && <p className="hint">{tg('kick.unsupported')}</p>}
                    {list.length === 0 ? (
                        <small className="hint">{tg('kick.empty')}</small>
                    ) : (
                        <ul className="rows">
                            {list.map((n) => (
                                <li key={n} className="row member-row">
                                    <div className="row-main">
                                        <div className="row-title row-title-one">{n}</div>
                                    </div>
                                    <Button
                                        disabled={busy}
                                        aria-label={tg('controllers.remove_named', { name: n })}
                                        onClick={() => save(list.filter((x) => x !== n))}
                                    >
                                        {tg('controllers.remove')}
                                    </Button>
                                </li>
                            ))}
                        </ul>
                    )}
                    <form
                        className="form"
                        onSubmit={(e) => {
                            e.preventDefault();
                            add();
                        }}
                    >
                        <label className="field">
                            <span>{tg('kick.placeholder')}</span>
                            <input value={name} maxLength={64} onChange={(e) => setName(e.target.value)} />
                        </label>
                        <Button tone="accent" type="submit" busy={busy} disabled={!name.trim()}>
                            {tg('kick.add')}
                        </Button>
                        <small className="hint">{tg('controllers_only.hint')}</small>
                    </form>
                </Sheet>
            )}
        </>
    );
}

/** 전용 멤버의 자동 강퇴 스위치: 명단에 없는 닉네임이 들어오면 내보낸다. 명단이 비면 켤 수 없다 */
export function MembersKick({ api, server }: { api: WebApi; server: Server }) {
    const { t: tg } = useTranslation('gui');
    const patch = usePatch(api, server);
    const [on, setOn] = useState(!!server.controllers_only);
    const [busy, setBusy] = useState(false);
    useEffect(() => setOn(!!server.controllers_only), [server.controllers_only]);
    if (!server.controllers_only_supported) return <small className="hint">{tg('kick.unsupported')}</small>;
    const empty = (server.controllers || []).length === 0;
    return (
        <label className="field field-row">
            <span>
                {tg('controllers_only.label')}
                <small className="hint">{empty ? tg('controllers_only.empty') : tg('controllers_only.hint')}</small>
            </span>
            <input
                type="checkbox"
                className="switch"
                checked={on}
                disabled={busy || (empty && !on)}
                onChange={async (e) => {
                    const checked = e.target.checked;
                    setBusy(true);
                    if (await patch({ controllers_only: checked })) setOn(checked);
                    setBusy(false);
                }}
            />
        </label>
    );
}
