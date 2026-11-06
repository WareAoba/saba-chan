import * as L from '@gui/components/discord/levels';
import type { CommandBoard, GuildPermissions, StairChip } from '@gui/components/discord/levels';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import { Dropdown } from './Dropdown';
import { Button, Segments } from './ui';
import type { ApiBody, WebApi } from '../types';

/** 저장 한 번을 맡기는 함수: 알림·다시 읽기는 부르는 쪽이 한다 */
export type PermissionWriter = (fn: () => Promise<ApiBody>) => Promise<void>;

const RISK_TONE: Record<string, string> = { safe: 'ok', caution: 'warn', danger: 'stop' };
const toneOf = (c: StairChip) => (c.kind === 'admin' ? 'info' : RISK_TONE[c.confirmRisk as string] || 'ok');
/** 명령 묶음: 조회 · 게임 조작 · 관리 (도움말·목록은 조회) — GUI `CommandList`와 같다 */
const GROUPS = ['query', 'command', 'admin'] as const;
const groupOf = (c: StairChip) => (c.kind === 'admin' ? 'admin' : c.kind === 'command' ? 'command' : 'query');

/**
 * 명령 목록 (GUI `CommandList`와 같은 규칙). 명령마다 "누구부터 쓸 수 있나"를 그 줄에서 고른다. 위 레벨은 아래 레벨의 명령을 모두 쓴다.
 * `instance`면 그 인스턴스 명령과 서버 전체 명령만 보인다. 디스코드 되묻기 단계는 디스코드 설정이라 보기만 한다 (PC 앱에서 바꾼다)
 */
interface CommandListProps {
    api: WebApi;
    guildId: string;
    perm: GuildPermissions;
    board: CommandBoard;
    instance: string;
    busy: boolean;
    write: PermissionWriter;
}

export function CommandList({ api, guildId, perm, board, instance, busy, write }: CommandListProps) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const ask = useApp((s) => s.ask);
    const [query, setQuery] = useState('');

    const levelName = (id: number) => L.ladderOf(perm).find((l) => l.id === id)?.name || t(`perm.level.${id}`);
    const levelTitle = (id: number) => t('perm.level_numbered', { n: L.levelNumber(perm, id), name: levelName(id) });
    const update = (patch: ApiBody) => write(() => api.permissionsUpdate(guildId, patch));
    const move = (chip: StairChip, level: number) => {
        // 대표 레벨이 같아도 다른 레벨인 인스턴스가 있으면 맞춘다
        if (busy || chip.fixed || level === L.BANNED || chip.items.every((i) => i.fixed || i.level === level)) return;
        update({ commandLevels: L.levelPatch(chip.items, level) });
    };

    const instances = board?.instances || [];
    const instanceName = (id: string) => instances.find((i) => i.id === id)?.name || id;
    const levels = L.ascending(perm);
    const usable = levels.filter((l) => l !== L.BANNED);
    const top = usable[usable.length - 1];
    const chips: StairChip[] = [
        { id: 'basic', title: t('perm.help_list'), level: usable[0], fixed: true, basic: true, items: [] },
        ...L.stairChips(board, instance),
    ];
    const label = (c: StairChip) => (c.instance ? `${c.title} · ${instanceName(c.instance)}` : c.title || '');
    const fromText = (l: number) => tg(l === top ? 'discord.perm_only' : 'discord.perm_from', { level: levelTitle(l) });
    // 요약: 레벨마다 쓸 수 있는 명령 수 (아래 레벨 것을 이어받는다). 인스턴스 꼬리표 줄은 세지 않는다
    const counted = chips.filter((c) => !c.instance);
    const summary = usable
        .map((l) => `${levelName(l)} ${counted.filter((c) => levels.indexOf(c.level) <= levels.indexOf(l)).length}`)
        .join(' · ');
    const q = query.trim().toLowerCase();
    const shown = q ? chips.filter((c) => label(c).toLowerCase().includes(q)) : chips;

    const preset = L.PRESETS.includes(perm.preset as string) ? (perm.preset as string) : 'default';
    const overrides = Object.keys(perm.commandLevels || {}).length;
    // 프리셋은 목록 전체를 한 번 채우는 틀: 옮긴 명령이 있으면 다시 맞춘다고 먼저 묻는다
    const pickPreset = async (p: string) => {
        if (busy || (p === preset && !overrides)) return;
        if (overrides) {
            const ok = await ask({
                title: t('perm.preset'),
                message: t('perm.preset_reset', { count: overrides, preset: t(`perm.preset_${p}`) }),
                confirmLabel: t('perm.preset_apply'),
            });
            if (!ok) return;
        }
        update({ preset: p });
    };
    const moved = L.overriddenCount(board);
    const line = L.controllerLine(board); // 사다리의 조작 가능 줄 (그 레벨인 명령이 줄을 따른다)

    const row = (c: StairChip) => {
        const name = label(c);
        const note = [
            c.fixed && (c.basic ? t('perm.help_hint') : t('perm.fixed_hint')),
            !c.fixed && c.overridden && tg('discord.perm_moved'),
            c.controllers && tg('discord.perm_controllers_tag'),
            c.controllers && line?.level === c.level && tg('discord.perm_line_follow'),
            instance !== L.ALL_INSTANCES && c.items.some((i) => !i.instance) && tg('discord.perm_server_wide_tag'),
            instance === L.ALL_INSTANCES &&
                !c.instance &&
                c.items.length > 1 &&
                tg('discord.perm_instances_count', { count: c.items.length }),
        ]
            .filter(Boolean)
            .join(' · ');
        return (
            <li key={c.id} className={`cmd-row${c.overridden ? ' is-moved' : ''}`}>
                <span className="cmd-name">
                    <span className={`dot tone-${toneOf(c)}`} aria-hidden="true" />
                    <span>
                        <strong>{name}</strong>
                        {note && <small className="hint">{note}</small>}
                    </span>
                </span>
                <Dropdown
                    value={c.levelMixed ? '' : String(c.level)}
                    ariaLabel={tg('discord.perm_who_for', { command: name })}
                    disabled={busy || c.fixed}
                    placeholder={tg('discord.perm_confirm_tier_mixed')}
                    onChange={(v: string) => move(c, Number(v))}
                    options={usable.map((l) => ({ value: String(l), label: fromText(l) }))}
                />
                <span className="cmd-risk hint">
                    {!c.basic && c.confirmRisk ? t(`perm.risk_${c.confirmRisk}`) : ''}
                </span>
                {c.overridden && !c.fixed ? (
                    <Button
                        disabled={busy}
                        onClick={() =>
                            update({
                                commandLevels: Object.fromEntries(
                                    c.items.flatMap((i) => (i.keys || [i.key]).map((k) => [k, null])),
                                ),
                            })
                        }
                    >
                        {t('perm.back_to_preset')}
                    </Button>
                ) : (
                    <span />
                )}
            </li>
        );
    };

    return (
        <div className="cmd-list-view">
            <div className="field">
                <span>{t('perm.preset')}</span>
                <Segments
                    value={preset}
                    onChange={pickPreset}
                    options={L.PRESETS.map((p) => ({ value: p, label: t(`perm.preset_${p}`) }))}
                />
                {moved > 0 && (
                    <div className="field-row">
                        <small className="hint">{t('perm.moved_count', { count: moved })}</small>
                        <Button disabled={busy} onClick={() => pickPreset(preset)}>
                            {t('perm.reset_to_preset')}
                        </Button>
                    </div>
                )}
            </div>
            <small className="hint">{tg('discord.perm_list_hint')}</small>
            <small className="hint">{t('perm.risk_pc_hint')}</small>
            <input
                className="search"
                type="search"
                value={query}
                placeholder={tg('discord.perm_search')}
                aria-label={tg('discord.perm_search')}
                onChange={(e) => setQuery(e.target.value)}
            />
            <small className="hint">{summary}</small>
            {GROUPS.map((g) => {
                const list = shown.filter((c) => groupOf(c) === g);
                if (!list.length) return null;
                return (
                    <section key={g} className="cmd-group" aria-label={tg(`discord.perm_group_${g}`)}>
                        <h4>{tg(`discord.perm_group_${g}`)}</h4>
                        <ul className="cmd-list">{list.map(row)}</ul>
                    </section>
                );
            })}
            {!shown.length && <small className="hint">{tg('discord.perm_search_none')}</small>}
        </div>
    );
}
