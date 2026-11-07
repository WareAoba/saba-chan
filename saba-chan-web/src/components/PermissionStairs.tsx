import type { CommandBoard, GuildPermissions, StairChip } from '@gui/components/discord/levels';
import * as L from '@gui/components/discord/levels';
import { usePointerDrag } from '@gui/components/discord/pointerDrag';
import type { CSSProperties, RefObject } from 'react';
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import type { ApiBody, WebApi } from '../types';
import { Icon } from './icons';
import { Button, Segments } from './ui';

/** 저장 한 번을 맡기는 함수: 알림·다시 읽기는 부르는 쪽이 한다 */
export type PermissionWriter = (fn: () => Promise<ApiBody>) => Promise<void>;

const RISK_TONE: Record<string, string> = { safe: 'ok', caution: 'warn', danger: 'stop' };
const toneOf = (c: StairChip) =>
    c.kind === 'control' ? 'control' : c.kind === 'admin' ? 'info' : RISK_TONE[c.confirmRisk as string] || 'ok';

/**
 * 칩이 다른 칸으로 옮겨지면 옛 자리에서 새 자리로 미끄러지게 한다 (FLIP, 앱 계단과 같다).
 * 자리는 그릴 때마다 다시 재고, 움직임은 `dep`(계단 값)이 바뀔 때만 준다
 */
function useChipFlip(ref: RefObject<HTMLElement | null>, dep: unknown) {
    const last = useRef(new Map<string, DOMRect>());
    const prevDep = useRef(dep);
    useLayoutEffect(() => {
        const root = ref.current;
        if (!root) return;
        const changed = prevDep.current !== dep;
        prevDep.current = dep;
        const reduce = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
        const next = new Map<string, DOMRect>();
        for (const el of root.querySelectorAll<HTMLElement>('[data-chip]')) {
            const box = el.getBoundingClientRect();
            next.set(el.dataset.chip as string, box);
            const prev = last.current.get(el.dataset.chip as string);
            if (!changed || reduce || !prev || !el.animate) continue;
            const dx = prev.left - box.left;
            const dy = prev.top - box.top;
            if (Math.abs(dx) < 1 && Math.abs(dy) < 1) continue;
            el.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: 'none' }], {
                duration: 260,
                easing: 'cubic-bezier(0.2, 0.8, 0.2, 1)',
            });
        }
        last.current = next;
    });
}

/**
 * 레벨 칸이 창에 남은 높이를 채우게 한다 (앱 `useFillHeight`와 같은 생각): 넘치는 칩은 칸 안에서 세로로 스크롤된다.
 * 웹은 창이 스크롤되므로 창 아래 끝까지를 준다. 칸이 세로로 쌓이는 좁은 화면(휴대폰)에서는 CSS가 높이를 정한다
 */
function useFillHeight(ref: RefObject<HTMLElement | null>, min = 260) {
    const [, bump] = useState(0);
    useLayoutEffect(() => {
        const el = ref.current;
        if (!el) return;
        if (getComputedStyle(el).display !== 'grid' || window.matchMedia?.('(max-width: 560px)').matches) {
            el.style.height = '';
            return;
        }
        const top = el.getBoundingClientRect().top + window.scrollY;
        const next = Math.max(min, Math.floor(window.innerHeight - top - 24));
        if (Math.abs(next - el.getBoundingClientRect().height) > 1) el.style.height = `${next}px`;
    });
    useEffect(() => {
        const on = () => bump((n) => n + 1);
        window.addEventListener('resize', on);
        return () => window.removeEventListener('resize', on);
    }, []);
}

/**
 * 레벨별 명령 계단 (앱 `CommandStairs`와 같은 규칙). 왼쪽이 낮은 레벨이고, 위 레벨은 아래 레벨의 명령을 모두 쓴다.
 * 금지 칸은 없다(금지면 아무것도 못 쓴다). 한 인스턴스 화면은 그 인스턴스 명령만, 모든 인스턴스 화면은 도움말·서버 전체 명령까지.
 * 디스코드 되묻기 단계는 디스코드 설정이라 보기만 한다 (PC 앱에서 바꾼다)
 */
interface PermissionStairsProps {
    api: WebApi;
    guildId: string;
    perm: GuildPermissions;
    board: CommandBoard;
    instance: string;
    busy: boolean;
    write: PermissionWriter;
}

export function PermissionStairs({ api, guildId, perm, board, instance, busy, write }: PermissionStairsProps) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const ask = useApp((s) => s.ask);
    const [selected, setSelected] = useState<string | null>(null);
    const ref = useRef<HTMLOListElement>(null);
    // 칩 끌기: 놓으면 dropRef가 옮긴다 (아래에서 계단 값을 읽은 뒤 채운다)
    const dropRef = useRef<(id: string, level: number) => void>(() => {});
    const chipDrag = usePointerDrag((id: string, to: string) => dropRef.current(id, Number(to)));
    const dragging = chipDrag.dragging;
    const over = chipDrag.over === null ? null : Number(chipDrag.over);
    // 옮긴 레벨은 초안이다 (사용자 지정 2026-11-07, 앱과 같다): 저장 막대에서 저장한다. 명령 키 → 레벨
    const [moves, setMoves] = useState<Record<string, number>>({});
    // biome-ignore lint/correctness/useExhaustiveDependencies: 데몬 값을 다시 읽으면(저장 뒤) 초안을 비운다
    useEffect(() => setMoves({}), [board]);
    // 칩은 저장된 값이 바뀔 때도, 초안에서 옮길 때도 미끄러진다
    // biome-ignore lint/correctness/useExhaustiveDependencies: board·moves 가 바뀔 때마다 새 표지
    const flipToken = useMemo(() => ({}), [board, moves]);
    useChipFlip(ref, flipToken);
    useFillHeight(ref);

    const levelName = (id: number) => L.ladderOf(perm).find((l) => l.id === id)?.name || t(`perm.level.${id}`);
    const levelTitle = (id: number) => t('perm.level_numbered', { n: L.levelNumber(perm, id), name: levelName(id) });
    const update = (patch: ApiBody) => write(() => api.permissionsUpdate(guildId, patch));
    const items = board?.items || [];
    const draftLevels = items.map((it) =>
        !it.fixed && moves[it.key as string] !== undefined ? moves[it.key as string] : it.level,
    );
    const shownBoard = L.draftBoard(board, draftLevels);
    const patch = L.boardPatch(board, draftLevels);
    const dirty = Object.keys(patch).length > 0;
    // 노란 테두리는 저장하지 않은 변경: 저장된 레벨과 초안 레벨이 다른 명령 (제자리로 돌려 놓으면 사라진다)
    const savedLevel = new Map(items.map((it) => [it.key as string, it.level]));
    const unsaved = (c: StairChip) => c.items.some((it) => savedLevel.get(it.key as string) !== it.level);
    const draftTo = (list: L.CommandItem[], level: (it: L.CommandItem) => number) =>
        setMoves((m) => ({
            ...m,
            ...Object.fromEntries(list.filter((it) => !it.fixed).map((it) => [it.key as string, level(it)])),
        }));
    const move = (chip: StairChip, level: number) => {
        // 대표 레벨이 같아도 다른 레벨인 인스턴스가 있으면 맞춘다
        if (busy || chip.fixed || level === L.BANNED || chip.items.every((i) => i.fixed || i.level === level)) return;
        draftTo(chip.items, () => level);
    };

    const instances = board?.instances || [];
    const instanceName = (id: string) => instances.find((i) => i.id === id)?.name || id;
    const levels = L.ascending(perm);
    const steps = levels.filter((l) => l !== L.BANNED);
    const oneInstance = instance !== L.ALL_INSTANCES;
    const groupControl = (list: StairChip[]) => L.groupControllers(list, tg('discord.perm_server_control'));
    const chips: StairChip[] = oneInstance
        ? groupControl(L.stairChips(shownBoard, instance).filter((c) => c.items.some((i) => i.instance)))
        : [
              // help·list는 금지가 아니면 누구나 쓴다 (바꿀 수 없다)
              { id: 'basic', title: t('perm.help_list'), level: steps[0], fixed: true, basic: true, items: [] },
              ...groupControl(L.stairChips(shownBoard, instance)),
          ];
    // 기본값으로 초기화: 보이는 명령을 프리셋 레벨로 (초안, 저장해야 반영)
    const resetToDefault = () =>
        draftTo(
            chips.flatMap((c) => c.items),
            (it) => it.preset as number,
        );
    dropRef.current = (id, level) => {
        const chip = chips.find((c) => c.id === id);
        if (chip) move(chip, level);
    };
    const current = chips.find((c) => c.id === selected);
    const label = (c: StairChip) => (c.instance ? `${c.title} · ${instanceName(c.instance)}` : c.title || '');
    // 게임(모듈·플러그인) 명령 칩은 명령 원문, 번역 이름은 말풍선과 화면 읽기 이름으로 (사용자 지정 2026-11-07).
    // 사바쨩 기능(시작·백업 등)과 묶음·도움말 칩은 번역 이름 그대로
    const chipText = (c: StairChip) => {
        const text = c.module && c.name ? c.name : c.title;
        return c.instance ? `${text} · ${instanceName(c.instance)}` : text;
    };
    // 칸 하나에 새로 오는 칩 수 (위 칸은 이것들을 모두 이어받는다)
    const gains: Record<number, number> = {};
    for (const c of chips.filter((c) => !c.instance)) gains[c.level] = (gains[c.level] || 0) + 1;
    let carried = 0;

    const preset = L.PRESETS.includes(perm.preset as string) ? (perm.preset as string) : 'default';
    const overrides = Object.keys(perm.commandLevels || {}).length;
    // 프리셋은 계단 전체를 한 번 채우는 틀: 옮긴 명령이 있으면 다시 맞춘다고 먼저 묻는다
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

    return (
        <div className="stairs-view">
            <div className="field">
                <span>{t('perm.preset')}</span>
                <Segments
                    value={preset}
                    onChange={pickPreset}
                    options={L.PRESETS.map((p) => ({ value: p, label: t(`perm.preset_${p}`) }))}
                />
                <div className="field-row">
                    <Button disabled={busy} onClick={resetToDefault}>
                        <Icon name="undo" size={16} /> {tg('discord.perm_ladder_reset')}
                    </Button>
                </div>
                {moved > 0 && (
                    <div className="field-row">
                        <small className="hint">{t('perm.moved_count', { count: moved })}</small>
                        <Button disabled={busy} onClick={() => pickPreset(preset)}>
                            {t('perm.reset_to_preset')}
                        </Button>
                    </div>
                )}
            </div>
            <small className="hint">{tg('discord.perm_stairs_hint')}</small>

            <ol
                ref={ref}
                className="stairs"
                style={{ '--steps': steps.length } as CSSProperties}
                aria-label={tg('discord.perm_stairs')}
            >
                {steps.map((level, i) => {
                    const here = chips.filter((c) => c.level === level);
                    const below = carried;
                    carried += gains[level] || 0;
                    return (
                        <li
                            key={level}
                            data-drop={level}
                            className={`stair${dragging !== null && over === level ? ' is-over' : ''}`}
                            style={{ '--step': i } as CSSProperties}
                            aria-label={levelTitle(level)}
                        >
                            <div className="stair-chips">
                                {below > 0 && (
                                    <span className="stair-carry">
                                        {tg('discord.perm_carry', { below: levelTitle(steps[i - 1]), count: below })}
                                    </span>
                                )}
                                {here.map((c) => (
                                    <button
                                        key={c.id}
                                        type="button"
                                        data-chip={c.id}
                                        aria-label={label(c)}
                                        title={label(c)}
                                        className={`cmd-chip${unsaved(c) ? ' is-changed' : ''}${
                                            c.fixed ? ' is-fixed' : ''
                                        }${selected === c.id ? ' is-selected' : ''}${
                                            dragging === c.id ? ' is-dragging' : ''
                                        }`}
                                        aria-pressed={selected === c.id}
                                        {...chipDrag.bind(c.id, !c.fixed)}
                                        onClick={() => setSelected(selected === c.id ? null : c.id)}
                                    >
                                        <span className={`dot tone-${toneOf(c)}`} aria-hidden="true" />
                                        <span className="cmd-chip-text">{chipText(c)}</span>
                                    </button>
                                ))}
                            </div>
                            <div className="stair-label">
                                <span className="stair-num">Lv {L.levelNumber(perm, level)}</span>
                                <strong>{levelName(level)}</strong>
                            </div>
                        </li>
                    );
                })}
            </ol>

            {current && (
                <section className="chip-edit" aria-label={label(current)}>
                    <strong>{label(current)}</strong>
                    {/* 디스코드 되묻기 단계는 디스코드 설정이라 PC 앱에서만 바꾼다 (원격 문지기) */}
                    {!current.basic && (current.confirmRisk || current.confirmMixed) && (
                        <small className="hint">
                            {tg('discord.perm_confirm_tier')}:{' '}
                            {current.confirmMixed
                                ? tg('discord.perm_confirm_tier_mixed')
                                : t(`perm.risk_${current.confirmRisk}`)}{' '}
                            · {t('perm.risk_pc_hint')}
                        </small>
                    )}
                    {current.fixed ? (
                        <small className="hint">{current.basic ? t('perm.help_hint') : t('perm.fixed_hint')}</small>
                    ) : (
                        <>
                            <small className="hint">{tg('discord.perm_from_level')}</small>
                            <fieldset className="level-steps" aria-label={tg('discord.perm_from_level')}>
                                {steps.map((l) => (
                                    <button
                                        key={l}
                                        type="button"
                                        className={`level-step${!current.levelMixed && l === current.level ? ' is-on' : ''}${
                                            levels.indexOf(l) > levels.indexOf(current.level) ? ' is-up' : ''
                                        }`}
                                        aria-pressed={!current.levelMixed && l === current.level}
                                        disabled={busy}
                                        onClick={() => move(current, l)}
                                    >
                                        {levelTitle(l)}
                                    </button>
                                ))}
                            </fieldset>
                            {current.controllers && (
                                <small className="hint">
                                    {[
                                        tg('discord.perm_controllers_tag'),
                                        line?.level === current.level && tg('discord.perm_line_follow'),
                                    ]
                                        .filter(Boolean)
                                        .join(' · ')}
                                </small>
                            )}
                            {oneInstance && current.items.some((i) => !i.instance) && (
                                <small className="hint">{tg('discord.perm_server_wide')}</small>
                            )}
                            {!oneInstance && !current.instance && current.items.length > 1 && (
                                <small className="hint">
                                    {tg('discord.perm_applies_to', { count: current.items.length })}
                                </small>
                            )}
                            {current.items.some((i) => !i.fixed && i.level !== i.preset) && (
                                <Button
                                    disabled={busy}
                                    onClick={() => draftTo(current.items, (it) => it.preset as number)}
                                >
                                    {t('perm.back_to_preset')}
                                </Button>
                            )}
                        </>
                    )}
                </section>
            )}

            <div className="stair-legend">
                {['safe', 'caution', 'danger'].map((r) => (
                    <span key={r}>
                        <span className={`dot tone-${RISK_TONE[r]}`} /> {t(`perm.risk_${r}`)}
                    </span>
                ))}
                <span>
                    <span className="dot tone-info" /> {tg('discord.kind_admin')}
                </span>
            </div>
            {dirty && (
                <div className="save-bar" role="status">
                    <span>{tg('discord.unsaved')}</span>
                    <Button disabled={busy} onClick={() => setMoves({})}>
                        {tg('discord.revert')}
                    </Button>
                    <Button tone="accent" busy={busy} onClick={() => update({ commandLevels: patch })}>
                        {tg('discord.save')}
                    </Button>
                </div>
            )}
        </div>
    );
}
