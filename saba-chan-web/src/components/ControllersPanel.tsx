import type { MembersByGuild } from '@gui/components/discord/types';
import { addController, type MemberChoice, memberChoices, removeController } from '@gui/components/servers/controllers';
import type { Controller } from '@gui/types/daemon';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import type { Server, WebApi } from '../types';
import { Dropdown } from './Dropdown';
import { Avatar, Button, Card, errorText, failed } from './ui';

/** 이 PC 에 이은 디스코드 서버(5곳까지)의 멤버. 멤버 목록은 느려서(최대 20초) 탭을 열 때 한 번만 받는다 */
function useMembers(api: WebApi): MemberChoice[] {
    const hostId = useApp((s) => s.session.hostId);
    const [members, setMembers] = useState<MemberChoice[]>([]);
    useEffect(() => {
        if (!hostId) return;
        let alive = true;
        const byGuild: MembersByGuild = {};
        api.relayListHostNodes(hostId, '')
            .catch(() => [])
            .then((nodes) => {
                const list = (Array.isArray(nodes) ? nodes : nodes?.nodes || []) as { guildId: string }[];
                for (const { guildId } of list.slice(0, 5)) {
                    api.relayListNodeMembers(String(guildId), '').then((m) => {
                        if (!alive || !Array.isArray(m?.members)) return;
                        byGuild[guildId] = m.members;
                        setMembers(memberChoices(byGuild));
                    });
                }
            });
        return () => {
            alive = false;
        };
    }, [api, hostId]);
    return members;
}

/** 인스턴스 "조작 가능": 시작·정지·재시작·백업 만들기를 할 수 있는 사람 명단 (GUI ControllersTab 과 같다) */
export function ControllersPanel({ api, server }: { api: WebApi; server: Server }) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const toast = useApp((s) => s.toast);
    const [list, setList] = useState<Controller[]>(server.controllers || []);
    const [name, setName] = useState('');
    const [discordId, setDiscordId] = useState('');
    const [busy, setBusy] = useState(false);
    const members = useMembers(api);
    const savedKey = JSON.stringify(server.controllers || []);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 데몬 값이 바뀌었을 때만 따라간다
    useEffect(() => {
        setList(server.controllers || []);
    }, [savedKey]);

    const save = async (next: Controller[]) => {
        setBusy(true);
        const res = await api.instanceUpdateSettings(server.id, { controllers: next });
        setBusy(false);
        if (failed(res)) {
            const gateOn = res?.fields?.some?.((f: { field?: string }) => f.field === 'controllers_only');
            toast(gateOn ? tg('controllers.err_gate_on') : errorText(t, res), 'error');
            return false;
        }
        setList(next);
        return true;
    };

    const add = async () => {
        const out = addController(list, name, discordId);
        if ('error' in out) return toast(tg(`controllers.err_${out.error}`), 'error');
        if (await save(out.list)) {
            setName('');
            setDiscordId('');
        }
    };
    const member = (id?: string) => members.find((m) => m.id === id);

    return (
        <div className="panel">
            <Card icon="members" tone="2" title={tg('shell.tab_controllers')} desc={tg('controllers.desc')}>
                {list.length === 0 ? (
                    <small className="hint">{tg('controllers.empty')}</small>
                ) : (
                    <ul className="rows">
                        {list.map((c, i) => {
                            const m = member(c.discord_id);
                            return (
                                <li key={c.name} className="row member-row">
                                    <div className="row-main">
                                        <div className="row-title row-title-one">{c.name}</div>
                                        <div className="row-sub row-title-one">
                                            {c.discord_id ? m?.label || c.discord_id : tg('controllers.not_linked')}
                                        </div>
                                    </div>
                                    {m && <Avatar url={m.avatarUrl} name={m.label} />}
                                    <Button disabled={busy} onClick={() => save(removeController(list, i))}>
                                        {tg('controllers.remove')}
                                    </Button>
                                </li>
                            );
                        })}
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
                        <span>{tg('controllers.name_placeholder')}</span>
                        <input value={name} maxLength={64} onChange={(e) => setName(e.target.value)} />
                    </label>
                    <div className="field">
                        <span>{tg('controllers.member_none')}</span>
                        <Dropdown
                            value={discordId}
                            ariaLabel={tg('controllers.member_none')}
                            onChange={(v) => setDiscordId(String(v))}
                            options={[
                                { value: '', label: tg('controllers.member_none') },
                                ...members.map((m) => ({ value: m.id, label: m.label })),
                            ]}
                        />
                    </div>
                    <Button tone="accent" type="submit" busy={busy} disabled={!name.trim()}>
                        {tg('controllers.add')}
                    </Button>
                    <small className="hint">{tg('controllers.note')}</small>
                </form>
            </Card>
        </div>
    );
}
