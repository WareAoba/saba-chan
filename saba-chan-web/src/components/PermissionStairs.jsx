import * as L from '@gui/components/discord/levels';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import { Button, Segments } from './ui';

const RISK_TONE = { safe: 'ok', caution: 'warn', danger: 'stop' };
const toneOf = (c) => (c.kind === 'admin' ? 'info' : RISK_TONE[c.risk] || 'ok');

/**
 * 칩이 다른 계단으로 옮겨지면 옛 자리에서 새 자리로 미끄러지게 한다 (FLIP, GUI 계단과 같다).
 * 자리는 그릴 때마다 다시 재고(스크롤·위 줄이 생겨도 옛 값이 남지 않게), 움직임은 `dep`(계단 값)이 바뀔 때만 준다
 */
function useChipFlip(ref, dep) {
    const last = useRef(new Map());
    const pinned = useRef(new Map()); // 놓은 칩: 다음 계단 값까지 놓은 자리를 옛 자리로 쓴다
    const prevDep = useRef(dep);
    useLayoutEffect(() => {
        const root = ref.current;
        if (!root) return;
        const changed = prevDep.current !== dep;
        prevDep.current = dep;
        const reduce = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
        const next = new Map();
        for (const el of root.querySelectorAll('[data-chip]')) {
            const box = el.getBoundingClientRect();
            next.set(el.dataset.chip, box);
            const prev = pinned.current.get(el.dataset.chip) || last.current.get(el.dataset.chip);
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
        if (changed) pinned.current.clear();
    });
    return pinned;
}

/**
 * 손가락·마우스로 칩을 끈다 (휴대폰은 HTML 끌어 놓기가 없어 pointer 이벤트로).
 * 6px 넘게 움직여야 끌기로 보고, 놓은 자리의 `data-level` 계단으로 옮긴다(`onDrop`이 옮겼으면 true).
 * 끈 뒤 같은 칩에 오는 click 하나만 무시한다. 화면이 닫히면 끌던 것을 버린다
 */
function useChipDrag({ onDrop, flip }) {
    const [drag, setDrag] = useState(null); // {id, over}
    const dragged = useRef(false);
    const latest = useRef(onDrop);
    latest.current = onDrop;
    const stop = useRef(null);
    useEffect(() => () => stop.current?.(), []);
    const stepAt = (x, y) => document.elementFromPoint?.(x, y)?.closest?.('[data-level]');

    const start = (e, chip) => {
        if (chip.fixed || (e.button !== undefined && e.button > 0)) return;
        stop.current?.();
        const el = e.currentTarget;
        const x0 = e.clientX;
        const y0 = e.clientY;
        let moving = false;
        dragged.current = false;
        const move = (ev) => {
            const dx = ev.clientX - x0;
            const dy = ev.clientY - y0;
            if (!moving && Math.hypot(dx, dy) < 6) return;
            moving = true;
            el.style.transform = `translate(${dx}px, ${dy}px)`;
            el.style.pointerEvents = 'none';
            const step = stepAt(ev.clientX, ev.clientY);
            const over = step ? Number(step.dataset.level) : null;
            setDrag((d) => (d?.id === chip.id && d.over === over ? d : { id: chip.id, over }));
        };
        const detach = () => {
            window.removeEventListener('pointermove', move);
            window.removeEventListener('pointerup', end);
            window.removeEventListener('pointercancel', end);
            el.style.transform = '';
            el.style.pointerEvents = '';
            stop.current = null;
        };
        const end = (ev) => {
            const box = moving ? el.getBoundingClientRect() : null;
            detach();
            setDrag(null);
            if (!moving) return;
            // 브라우저가 끈 칩에 click 을 보낼 수도, 안 보낼 수도 있다: 바로 뒤 click 하나만 무시한다
            dragged.current = true;
            setTimeout(() => {
                dragged.current = false;
            }, 0);
            if (ev.type !== 'pointerup') return;
            const step = stepAt(ev.clientX, ev.clientY);
            // 놓은 자리에서 새 계단으로 미끄러지게 (옮겼을 때만)
            if (step && latest.current(chip, Number(step.dataset.level))) flip.current.set(chip.id, box);
        };
        stop.current = () => {
            detach();
            setDrag(null);
        };
        window.addEventListener('pointermove', move);
        window.addEventListener('pointerup', end);
        window.addEventListener('pointercancel', end);
    };
    /** 끌기 바로 뒤에 오는 click 이면 true */
    const consumeDrag = () => dragged.current;
    return { drag, start, consumeDrag };
}

/**
 * 레벨별 명령 계단 (GUI `CommandStairs`와 같은 규칙). 위가 높은 레벨이고, 위 레벨은 아래 레벨의 명령을 모두 쓴다.
 * `instance`면 그 인스턴스 명령과 서버 전체 명령만 보인다
 */
export function PermissionStairs({ api, guildId, perm, board, instance, busy, write }) {
    const { t } = useTranslation();
    const ask = useApp((s) => s.ask);
    const [selected, setSelected] = useState(null);
    const ref = useRef(null);
    const flip = useChipFlip(ref, board);

    const levelName = (id) => L.ladderOf(perm).find((l) => l.id === id)?.name || t(`perm.level.${id}`);
    const levelTitle = (id) => t('perm.level_numbered', { n: L.levelNumber(perm, id), name: levelName(id) });
    const update = (patch) => write(() => api.permissionsUpdate(guildId, patch));
    const move = (chip, level) => {
        // 대표 레벨이 같아도 다른 레벨인 인스턴스가 있으면 맞춘다
        if (busy || chip.fixed || level === L.BANNED || chip.items.every((i) => i.fixed || i.level === level))
            return false;
        update({ commandLevels: L.levelPatch(chip.items, level) });
        return true;
    };
    const { drag, start, consumeDrag } = useChipDrag({ onDrop: move, flip });

    const instances = board?.instances || [];
    const instanceName = (id) => instances.find((i) => i.id === id)?.name || id;
    const levels = L.ascending(perm);
    const lowest = levels.find((l) => l !== L.BANNED);
    const chips = [
        { id: 'basic', title: t('perm.help_list'), level: lowest, fixed: true, basic: true, items: [] },
        ...L.stairChips(board, instance),
    ];
    const current = chips.find((c) => c.id === selected);
    const label = (c) => (c.instance ? `${c.title} · ${instanceName(c.instance)}` : c.title);
    const gains = {};
    for (const c of chips.filter((c) => !c.instance)) gains[c.level] = (gains[c.level] || 0) + 1;
    const carried = {};
    let total = 0;
    for (const l of levels) {
        carried[l] = total;
        total += gains[l] || 0;
    }

    const preset = L.PRESETS.includes(perm.preset) ? perm.preset : 'default';
    const overrides = Object.keys(perm.commandLevels || {}).length;
    // 프리셋은 계단 전체를 한 번 채우는 틀: 옮긴 명령이 있으면 다시 맞춘다고 먼저 묻는다
    const pickPreset = async (p) => {
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

    return (
        <div className="stairs-view">
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
            <small className="hint">{t('perm.stairs_hint')}</small>

            <ol ref={ref} className="stairs" aria-label={t('perm.stairs')}>
                {[...levels].reverse().map((level) => {
                    const here = chips.filter((c) => c.level === level);
                    const step = levels.indexOf(level);
                    return (
                        <li
                            key={level}
                            data-level={level}
                            className={`stair${level === L.BANNED ? ' is-banned' : ''}${
                                drag && drag.over === level && level !== L.BANNED ? ' is-over' : ''
                            }`}
                            style={{ '--step': step }}
                            aria-label={levelTitle(level)}
                        >
                            <div className="stair-label">
                                <span className="stair-num">Lv {L.levelNumber(perm, level)}</span>
                                <strong>{levelName(level)}</strong>
                            </div>
                            <div className="stair-chips">
                                {level !== L.BANNED && carried[level] > 0 && (
                                    <span className="stair-carry">
                                        {t('perm.carry', {
                                            below: levelTitle(levels[step - 1]),
                                            count: carried[level],
                                        })}
                                    </span>
                                )}
                                {level === L.BANNED && <span className="hint">{t('perm.level_none')}</span>}
                                {here.map((c) => (
                                    <button
                                        key={c.id}
                                        type="button"
                                        data-chip={c.id}
                                        className={`cmd-chip${c.overridden ? ' is-moved' : ''}${
                                            c.fixed ? ' is-fixed' : ''
                                        }${selected === c.id ? ' is-selected' : ''}${
                                            drag?.id === c.id ? ' is-dragging' : ''
                                        }`}
                                        aria-pressed={selected === c.id}
                                        onPointerDown={(e) => start(e, c)}
                                        onClick={() => {
                                            if (consumeDrag()) return;
                                            setSelected(selected === c.id ? null : c.id);
                                        }}
                                    >
                                        <span className={`dot tone-${toneOf(c)}`} aria-hidden="true" />
                                        {label(c)}
                                    </button>
                                ))}
                            </div>
                        </li>
                    );
                })}
            </ol>

            {current && (
                <section className="chip-edit" aria-label={label(current)}>
                    <strong>{label(current)}</strong>
                    {current.fixed ? (
                        <small className="hint">{current.basic ? t('perm.help_hint') : t('perm.fixed_hint')}</small>
                    ) : (
                        <>
                            <small className="hint">{t('perm.from_level')}</small>
                            <fieldset className="level-steps" aria-label={t('perm.from_level')}>
                                {levels
                                    .filter((l) => l !== L.BANNED)
                                    .map((l) => (
                                        <button
                                            key={l}
                                            type="button"
                                            className={`level-step${l === current.level ? ' is-on' : ''}${
                                                levels.indexOf(l) > levels.indexOf(current.level) ? ' is-up' : ''
                                            }`}
                                            aria-pressed={l === current.level}
                                            disabled={busy}
                                            onClick={() => move(current, l)}
                                        >
                                            {levelTitle(l)}
                                        </button>
                                    ))}
                            </fieldset>
                            {instance !== L.ALL_INSTANCES && current.items.some((i) => !i.instance) && (
                                <small className="hint">{t('perm.server_wide')}</small>
                            )}
                            {current.items.some((i) => i.overridden) && (
                                <Button
                                    disabled={busy}
                                    onClick={() =>
                                        update({
                                            commandLevels: Object.fromEntries(
                                                current.items.flatMap((i) => (i.keys || [i.key]).map((k) => [k, null])),
                                            ),
                                        })
                                    }
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
                    <span className="dot tone-info" /> {t('perm.kind_admin')}
                </span>
                <span>
                    <span className="cmd-chip is-moved is-legend">{t('perm.legend_moved')}</span>
                </span>
            </div>
        </div>
    );
}
