import clsx from 'clsx';
import { useEffect, useLayoutEffect, useState } from 'react';
import type { CSSProperties, RefObject } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaFail } from '../ui/SabaArt';
import { Segmented } from '../ui/StatusChip';
import { SaveBar } from '../ui/SaveBar';
import * as L from './levels';
import type { DiscordNode } from './types';
import { useStairsDraft } from './useStairsDraft';
import { LevelSelect, PermState } from './PermissionParts';
import type { PermissionsState } from './PermissionsTab';

// ── 명령: 계단. 칩은 "이 레벨부터 쓸 수 있다"는 자리에 있고, 위 계단은 아래 계단의 명령을 모두 쓴다 ──
/**
 * 레벨 칸이 화면에 남은 높이만 채우게 한다: 페이지는 스크롤되지 않고, 넘치는 칩은 칸 안에서 스크롤된다.
 * 가장 가까운 스크롤 조상의 보이는 높이에서 칸 아래위 내용을 빼고 남는 만큼을 준다 (창 크기·편집 창이 바뀔 때마다 다시 잰다)
 */
function useFillHeight(ref: RefObject<HTMLElement | null>, min = 220) {
    const [, bump] = useState(0);
    useLayoutEffect(() => {
        const el = ref.current;
        if (!el) return;
        let scroller = el.parentElement;
        let child: HTMLElement = el;
        while (scroller && !/(auto|scroll)/.test(getComputedStyle(scroller).overflowY)) {
            child = scroller;
            scroller = scroller.parentElement;
        }
        if (!scroller) return;
        // 스크롤 칸의 바로 아래 감싸개는 min-height로 늘어나 있을 수 있다: 그 안 내용의 끝과 아래 여백으로 잰다
        const padOf = (node: Element) => Number.parseFloat(getComputedStyle(node).paddingBottom) || 0;
        const box = scroller.getBoundingClientRect();
        const bottom = box.top + scroller.clientTop + scroller.clientHeight - padOf(scroller) - padOf(child);
        const parts = child === el ? [el] : [...child.children];
        const contentBottom =
            Math.max(...parts.map((n) => n.getBoundingClientRect().bottom)) + scroller.scrollTop;
        const cur = el.getBoundingClientRect().height;
        const next = Math.max(min, Math.floor(cur + bottom - contentBottom));
        if (Math.abs(next - cur) > 1) el.style.height = `${next}px`;
    });
    useEffect(() => {
        const on = () => bump((n) => n + 1);
        window.addEventListener('resize', on);
        return () => window.removeEventListener('resize', on);
    }, []);
}

interface CommandStairsProps {
    node: DiscordNode;
    permissions: PermissionsState;
    instance?: string;
}

export function CommandStairs({ node, permissions, instance = L.ALL_INSTANCES }: CommandStairsProps) {
    const { t } = useTranslation('gui');
    const { perm, board, loading, error, busy, load, write } = permissions;
    const [view, setView] = useState(instance);
    const [selected, setSelected] = useState<string | null>(null); // 칩 id
    const [presetDraft, setPresetDraft] = useState<string | null>(null); // 명령별 레벨을 덮을 프리셋 (확인 전)
    // 옮긴 레벨은 초안이다 (사용자 지정 2026-11-07): 저장 막대에서 저장한다
    const {
        ref: stairsRef,
        dropRef,
        chipDrag,
        dragging,
        over,
        setMoves,
        shownBoard,
        patch,
        dirty,
        unsaved,
        draftTo,
        move,
    } = useStairsDraft<HTMLOListElement>(board, busy);
    useFillHeight(stairsRef);
    if (!perm) return <PermState loading={loading} error={error} onRetry={load} />;
    if (!board)
        return <SabaFail message={t('discord.perm_err_generic')} onRetry={load} retryLabel={t('discord.reload')} />;

    const guildId = node.guildId;
    const update = (patch: ApiBody) => write(() => window.api.permissionsUpdate(guildId, patch));
    const preset = L.PRESETS.includes(perm.preset as string) ? (perm.preset as string) : 'default';
    // 프리셋은 한 번 적용하는 틀이다: 명령별 레벨이 있으면 다시 맞춘다고 먼저 묻는다 (지금 프리셋을 다시 눌러도)
    const overrides = Object.keys(perm.commandLevels || {}).length;
    const pickPreset = (p: string) => {
        if (busy || (p === preset && !overrides)) return;
        if (overrides) setPresetDraft(p);
        else update({ preset: p });
    };
    const instances = board.instances || [];
    const shownView = view === L.ALL_INSTANCES || instances.some((i) => i.id === view) ? view : L.ALL_INSTANCES;
    const instanceName = (id: string) => instances.find((i) => i.id === id)?.name || id;
    const levels = L.ascending(perm);
    const steps = levels.filter((l) => l !== L.BANNED); // 금지 칸은 그리지 않는다 (금지면 아무 명령도 못 쓴다)
    const lowest = levels.find((l) => l !== L.BANNED);
    // 한 인스턴스의 메뉴: 그 인스턴스 명령만 보인다 (서버 전체 명령·도움말은 모든 인스턴스 보기에서 정한다)
    const oneInstance = instance !== L.ALL_INSTANCES;
    const groupControl = (list: L.StairChip[]) => L.groupControllers(list, t('discord.perm_server_control'));
    const chips: L.StairChip[] = oneInstance
        ? groupControl(L.stairChips(shownBoard, shownView).filter((c) => c.items.some((i) => i.instance)))
        : [
              // help·list는 금지가 아니면 누구나 쓴다 (바꿀 수 없다)
              {
                  id: 'basic',
                  title: t('discord.perm_help_list'),
                  level: lowest as number,
                  fixed: true,
                  basic: true,
                  items: [],
              },
              ...groupControl(L.stairChips(shownBoard, shownView)),
          ];
    const current = chips.find((c) => c.id === selected);
    // 기본값으로 초기화: 보이는 명령을 프리셋 레벨로 (초안, 저장해야 반영)
    const resetToDefault = () => draftTo(chips.flatMap((c) => c.items), (it) => it.preset as number);
    dropRef.current = (id, level) => {
        const chip = chips.find((c) => c.id === id);
        if (chip && level !== L.BANNED) move(chip, level);
    };
    const setRisk = async (chip: L.StairChip, risk: string) => {
        if (busy || chip.items.every((i) => (i.confirmRisk || i.risk) === risk)) return;
        await write(() => window.api.discordSettingsSet(guildId, { commandRisks: L.riskPatch(chip.items, risk) }));
    };
    const label = (chip: L.StairChip) =>
        chip.instance ? `${chip.title} · ${instanceName(chip.instance)}` : chip.title;
    // 게임(모듈·플러그인) 명령 칩은 명령 원문, 번역 이름은 말풍선과 화면 읽기 이름으로 (사용자 지정 2026-11-07).
    // 바닐라 게임에 없는 사바쨩 기능(시작·백업 등)과 묶음·도움말 칩은 번역 이름 그대로
    const chipText = (chip: L.StairChip) => {
        const text = chip.module && chip.name ? chip.name : chip.title;
        return chip.instance ? `${text} · ${instanceName(chip.instance)}` : text;
    };
    // 계단 하나에 새로 오는 칩 수 (위 계단은 이것들을 모두 이어받는다)
    const gains: Record<number, number> = {};
    for (const c of chips.filter((c) => !c.instance)) gains[c.level] = (gains[c.level] || 0) + 1;
    let carried = 0;

    return (
        <div className="dc-tab">
            {/* 사용 모드 줄(InstanceAccess)과 같은 모양: 제목·선택은 왼쪽, 되돌리기 단추는 오른쪽 끝 */}
            <div className="access-bar">
                <span className="access-bar-title">
                    <Icon name="command" size="sm" /> {t('discord.perm_preset')}
                </span>
                <Segmented
                    ariaLabel={t('discord.perm_preset')}
                    value={preset}
                    onChange={pickPreset}
                    items={L.PRESETS.map((p) => ({ id: p, label: t(`discord.preset_${p}`) }))}
                />
                <span className="access-bar-end">
                    {L.overriddenCount(board) > 0 && (
                        <>
                            <span className="muted access-bar-note">
                                {t('discord.perm_moved_count', { count: L.overriddenCount(board) })}
                            </span>
                            <button
                                type="button"
                                className="btn btn-ghost btn-sm"
                                disabled={busy}
                                onClick={() => pickPreset(preset)}
                            >
                                {t('discord.perm_reset_to_preset')}
                            </button>
                        </>
                    )}
                    <button type="button" className="btn btn-ghost btn-sm" disabled={busy} onClick={resetToDefault}>
                        <Icon name="undo" size="sm" /> {t('discord.perm_ladder_reset')}
                    </button>
                </span>
            </div>
            {presetDraft && (
                <div className="notice tone-warn" role="alert">
                    <Icon name="alertTriangle" size="sm" />
                    <span>
                        {t('discord.perm_preset_reset', {
                            count: overrides,
                            preset: t(`discord.preset_${presetDraft}`),
                        })}
                    </span>
                    <button
                        type="button"
                        className="btn btn-primary btn-sm"
                        disabled={busy}
                        onClick={async () => {
                            const p = presetDraft;
                            setPresetDraft(null);
                            await update({ preset: p });
                        }}
                    >
                        {t('discord.perm_preset_apply')}
                    </button>
                    <button type="button" className="btn btn-ghost btn-sm" onClick={() => setPresetDraft(null)}>
                        {t('modals.cancel')}
                    </button>
                </div>
            )}
            <p className="dc-hint">{t('discord.perm_stairs_hint')}</p>
            {instance === L.ALL_INSTANCES && instances.length > 1 && (
                <div className="dc-field-inline">
                    <span className="muted">{t('discord.perm_view')}</span>
                    <CustomDropdown
                        className="dc-level-select"
                        value={shownView}
                        ariaLabel={t('discord.perm_view')}
                        onChange={(v) => {
                            setView(v);
                            setSelected(null);
                        }}
                        options={[
                            { value: L.ALL_INSTANCES, label: t('discord.perm_view_all') },
                            ...instances.map((i) => ({ value: i.id, label: i.name })),
                        ]}
                    />
                </div>
            )}

            <div className="dc-stairs-scroll">
                <ol
                    ref={stairsRef}
                    className="dc-stairs"
                    style={{ '--steps': steps.length } as CSSProperties}
                    aria-label={t('discord.perm_stairs')}
                >
                    {steps.map((level, i) => {
                        const here = chips.filter((c) => c.level === level);
                        const below = carried;
                        carried += gains[level] || 0;
                        const droppable = dragging !== null;
                        return (
                            <li
                                key={level}
                                className={clsx('dc-step', `level-${L.levelKey(level)}`, {
                                    'is-over': over === level && droppable,
                                })}
                                style={{ '--step': i } as CSSProperties}
                                aria-label={L.levelTitle(t, perm, level)}
                                data-drop={level}
                            >
                                <div className="dc-step-chips">
                                    {below > 0 && (
                                        <span className="dc-step-carry">
                                            {t('discord.perm_carry', {
                                                below: L.levelTitle(t, perm, steps[i - 1]),
                                                count: below,
                                            })}
                                        </span>
                                    )}
                                    {here.map((c) => (
                                        <button
                                            key={c.id}
                                            type="button"
                                            data-chip={c.id}
                                            aria-label={label(c)}
                                            data-tooltip={label(c)}
                                            className={clsx('dc-cmd-chip', `kind-${c.kind || 'basic'}`, {
                                                'is-changed': unsaved(c),
                                                'is-moved': c.overridden,
                                                'is-fixed': c.fixed,
                                                'is-selected': selected === c.id,
                                                'is-dragging': dragging === c.id,
                                            })}
                                            aria-pressed={selected === c.id}
                                            {...chipDrag.bind(c.id, !c.fixed)}
                                            onClick={() => setSelected(selected === c.id ? null : c.id)}
                                        >
                                            <span
                                                className={clsx('dc-dot', `tone-${L.chipTone(c)}`)}
                                                aria-hidden="true"
                                            />
                                            {c.fixed && <Icon name="pin" size="xs" />}
                                            <span>{chipText(c)}</span>
                                        </button>
                                    ))}
                                </div>
                                <div className="dc-step-label">
                                    <span className="dc-level-num">Lv {L.levelNumber(perm, level)}</span>
                                    <strong>{L.levelLabel(t, perm, level)}</strong>
                                </div>
                            </li>
                        );
                    })}
                </ol>
            </div>

            {current && (
                <section className="dc-card dc-chip-edit" aria-label={label(current)}>
                    <div className="dc-card-head">
                        <strong>{label(current)}</strong>
                        {current.risk && (
                            <span
                                className={clsx('dc-badge', `tone-${L.RISK_TONE[current.confirmRisk as string] || 'ok'}`)}
                            >
                                {current.kind === 'admin'
                                    ? t('discord.kind_admin')
                                    : current.confirmMixed
                                      ? t('discord.perm_confirm_tier_mixed')
                                      : t(`discord.risk_${current.confirmRisk}`)}
                            </span>
                        )}
                    </div>
                    {!current.basic && (
                        <div className="dc-field-row">
                            <span className="dc-field-label">
                                <strong>{t('discord.perm_confirm_tier')}</strong>
                                <small>
                                    {t('discord.perm_confirm_tier_hint')}
                                    {current.confirmMixed
                                        ? ` ${t('discord.perm_confirm_tier_mixed_hint')}`
                                        : current.confirmRisk !== current.risk &&
                                          ` ${t('discord.perm_confirm_tier_default', { risk: t(`discord.risk_${current.risk}`) })}`}
                                </small>
                            </span>
                            <Segmented
                                ariaLabel={t('discord.perm_confirm_tier')}
                                value={current.confirmRisk as string}
                                onChange={(r) => setRisk(current, r)}
                                items={L.RISKS.map((r) => ({ id: r, label: t(`discord.risk_${r}`) }))}
                            />
                        </div>
                    )}
                    {current.fixed ? (
                        <p className="dc-hint">
                            {current.basic ? t('discord.perm_commands_help_hint') : t('discord.perm_fixed_hint')}
                        </p>
                    ) : (
                        <>
                            <span className="muted">{t('discord.perm_from_level')}</span>
                            <fieldset className="dc-level-steps" aria-label={t('discord.perm_from_level')}>
                                {levels
                                    .filter((l) => l !== L.BANNED)
                                    .map((l) => (
                                        <button
                                            key={l}
                                            type="button"
                                            className={clsx('dc-level-step', {
                                                'is-on': !current.levelMixed && l === current.level,
                                                'is-up': levels.indexOf(l) > levels.indexOf(current.level),
                                            })}
                                            aria-pressed={!current.levelMixed && l === current.level}
                                            disabled={busy}
                                            onClick={() => move(current, l)}
                                        >
                                            {L.levelTitle(t, perm, l)}
                                        </button>
                                    ))}
                            </fieldset>
                            {current.controllers && (
                                <p className="dc-hint">
                                    {[
                                        t('discord.perm_controllers_tag'),
                                        L.controllerLine(board)?.level === current.level && t('discord.perm_line_follow'),
                                    ]
                                        .filter(Boolean)
                                        .join(' · ')}
                                </p>
                            )}
                            {shownView !== L.ALL_INSTANCES && current.items.some((i) => !i.instance) && (
                                <p className="dc-hint">{t('discord.perm_server_wide')}</p>
                            )}
                            {shownView === L.ALL_INSTANCES && !current.instance && current.items.length > 1 && (
                                <p className="dc-hint">
                                    {t('discord.perm_applies_to', { count: current.items.length })}
                                </p>
                            )}
                            {current.items.some((i) => !i.fixed && i.level !== i.preset) && (
                                <button
                                    type="button"
                                    className="btn btn-ghost btn-sm"
                                    disabled={busy}
                                    onClick={() => draftTo(current.items, (it) => it.preset as number)}
                                >
                                    {t('discord.perm_back_to_preset')}
                                </button>
                            )}
                        </>
                    )}
                </section>
            )}

            <div className="dc-legend">
                <span>
                    <span className="dc-dot tone-ok" /> {t('discord.risk_safe')}
                </span>
                <span>
                    <span className="dc-dot tone-warn" /> {t('discord.risk_caution')}
                </span>
                <span>
                    <span className="dc-dot tone-stop" /> {t('discord.risk_danger')}
                </span>
                <span>
                    <span className="dc-dot tone-info" /> {t('discord.kind_admin')}
                </span>
            </div>

            <div className="dc-field-row">
                <span className="dc-field-label">
                    <strong>{t('discord.perm_chatbot_cap')}</strong>
                    <small>{t('discord.perm_chatbot_cap_hint')}</small>
                </span>
                <LevelSelect
                    value={typeof perm.chatbotCap === 'number' ? perm.chatbotCap : 0}
                    levels={L.grantable(perm)}
                    perm={perm}
                    label={t('discord.perm_chatbot_cap')}
                    disabled={busy}
                    onChange={(level) => update({ chatbotCap: level })}
                />
            </div>
            <SaveBar dirty={dirty} saving={busy} onRevert={() => setMoves({})} onSave={() => update({ commandLevels: patch })} />
        </div>
    );
}

