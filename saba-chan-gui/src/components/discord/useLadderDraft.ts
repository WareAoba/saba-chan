import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useFlip } from './flip';
import * as L from './levels';
import { usePointerDrag } from './pointerDrag';

export const ladderKeyOf = (level: L.LadderLevel) => String(level.id !== undefined ? `id-${level.id}` : level.key);

/**
 * 레벨 편집 초안 (앱 `LadderTab`·웹 `LadderEditor`가 같이 쓴다, 그리는 모양은 화면마다 다르다).
 * 카드를 끌어 순서를 바꾸고(놓으면 카드들이 새 자리로 미끄러진다), 칩을 끌어 다른 레벨로 옮기고,
 * 권한 맞바꾸기(사용자 지정 2026-11-07)는 칸 두 개를 고르면 두 레벨의 명령이 자리를 바꾼다.
 * 조작 가능 줄은 맞바꾸기 차례에서 자리를 낸다. `chipSelector`: 접힌 칩을 셀 때 찾는 칩 (화면마다 CSS 이름이 다르다)
 */
export function useLadderDraft<T extends HTMLElement>(
    perm: L.GuildPermissions | null | undefined,
    board: L.CommandBoard | null,
    chipSelector: string,
) {
    const saved = L.ladderOf(perm);
    const [draft, setDraft] = useState(saved);
    const [editing, setEditing] = useState<number | null>(null); // 이름을 고치는 칸
    const line = L.controllerLine(board);
    // 명령 옮긴 차례 (맞바꾸기·줄 옮기기). 줄 자리는 이 차례에서 나온다
    const [steps, setSteps] = useState<L.CommandStep[]>([]);
    const [choosing, setChoosing] = useState(false);
    const [picked, setPicked] = useState<number | null>(null);
    // 칩 끌기로 옮긴 명령: 명령 키 → 레벨 (맞바꾸기 뒤에 덮는다)
    const [moves, setMoves] = useState<Record<string, number>>({});
    const listRef = useRef<T>(null);
    const flip = useFlip(listRef);
    // 카드 끌기: key·칸 값은 사다리 자리(위에서부터 0)
    const blockDrag = usePointerDrag((from: string, to: string) => {
        flip();
        setDraft((d) => L.moveLevel(d, Number(from), Number(to)));
    });
    // 칩 끌기: key는 칩 id, 칸 값은 사다리 자리
    const chipDropRef = useRef<(id: string, at: number) => void>(() => {});
    const chipDrag = usePointerDrag((id: string, to: string) => chipDropRef.current(id, Number(to)));

    // 데몬 값이 바뀌면(저장·다시 읽기) 초안을 그 값으로
    useEffect(() => setDraft(L.ladderOf(perm)), [perm]);
    // 데몬 값을 다시 읽으면(저장 뒤) 맞바꾸기 초안을 비운다
    // biome-ignore lint/correctness/useExhaustiveDependencies: board 가 바뀔 때만 비운다
    useEffect(() => {
        setSteps([]);
        setMoves({});
    }, [board]);

    const patch = L.ladderPatch(draft);
    const ladderChanged = JSON.stringify(patch) !== JSON.stringify(L.ladderPatch(saved));
    const lineLevel = L.draftLine(line, draft, steps);
    const lineAt = line && lineLevel !== null ? L.lineAt(saved, draft, lineLevel) : null;
    const lineTarget = lineAt === null ? null : (draft[lineAt].id as number);
    const items = board?.items || [];
    const levels = L.draftLevels(board, draft, steps, line, lineTarget).map((level, i) =>
        items[i] && !items[i].fixed && moves[items[i].key as string] !== undefined ? moves[items[i].key as string] : level,
    );
    const commandLevels = L.boardPatch(board, levels);
    const commandsChanged = Object.keys(commandLevels).length > 0;
    // 레벨 편집은 사바쨩 기능만 보인다 (사용자 지정 2026-11-07): 인스턴스마다 다른 게임(모듈) 명령과
    // 인스턴스마다 정하는 조작 가능(서버 조작)은 인스턴스 권한 화면에서 정한다. 맞바꾸기는 모든 명령을 그대로 옮긴다
    const chips = L.stairChips(L.draftBoard(board, levels)).filter((c) => !c.module && !c.controllers);
    chipDropRef.current = (id, at) => {
        const chip = chips.find((c) => c.id === id);
        const target = draft[at];
        if (!chip || chip.fixed || !target || !L.swappable(target)) return;
        const level = target.id as number;
        flip();
        setMoves((m) => ({
            ...m,
            ...Object.fromEntries(chip.items.filter((i) => !i.fixed).map((i) => [i.key as string, level])),
        }));
    };
    // 노란 테두리는 "저장하지 않은 변경" (사용자 지정 2026-11-07): 저장된 레벨과 초안 레벨이 다른 명령
    const savedLevel = new Map(items.map((it) => [it.key as string, it.level]));

    // 칩은 숨기지 않는다. 카드가 칩 세 줄을 넘을 때만 접고 "+N"을 누르면 그 카드의 칩을 모두 편다
    const chipBoxes = useRef(new Map<string, HTMLElement>());
    const [hidden, setHidden] = useState<Record<string, number>>({});
    const [expanded, setExpanded] = useState<Record<string, boolean>>({});
    useLayoutEffect(() => {
        // 끄는 동안은 세지 않는다: 손을 따라 나간 칩을 접힌 칩으로 세어 "+1"이 뜨지 않게
        if (chipDrag.dragging !== null) return;
        const next: Record<string, number> = {};
        for (const [k, box] of chipBoxes.current) {
            if (expanded[k]) continue;
            // 칸 위에서 잰 칩 자리가 보이는 높이를 넘으면 접힌 칩이다
            const top = box.getBoundingClientRect().top;
            const limit = box.clientHeight;
            const n = [...box.querySelectorAll<HTMLElement>(chipSelector)].filter(
                (el) => el.getBoundingClientRect().top - top >= limit,
            ).length;
            if (n > 0) next[k] = n;
        }
        if (JSON.stringify(next) !== JSON.stringify(hidden)) setHidden(next);
    });

    const instanceName = (id?: string) => board?.instances?.find((i) => i.id === id)?.name || id;
    return {
        saved,
        draft,
        setDraft,
        editing,
        setEditing,
        choosing,
        picked,
        listRef,
        blockDrag,
        chipDrag,
        dragging: blockDrag.dragging === null ? null : Number(blockDrag.dragging),
        over: blockDrag.over === null ? null : Number(blockDrag.over),
        chipOver: chipDrag.over === null ? null : Number(chipDrag.over),
        chips,
        unsaved: (c: L.StairChip) => c.items.some((it) => savedLevel.get(it.key as string) !== it.level),
        chipLabel: (c: L.StairChip) => (c.instance ? `${c.title} · ${instanceName(c.instance)}` : c.title || c.name),
        /** 칩 칸을 잴 수 있게 적어 둔다 (ref 콜백) */
        chipBox: (k: string) => (el: HTMLElement | null) => {
            if (el) chipBoxes.current.set(k, el);
            else chipBoxes.current.delete(k);
        },
        hidden,
        expanded,
        expand: (k: string) => setExpanded((x) => ({ ...x, [k]: true })),
        // 마지막 맞바꾸기 (두 칸이 아직 초안에 있을 때만 알린다)
        lastSwap: L.activeSwaps(steps, draft).at(-1) ?? null,
        removed: saved.some((s) => !draft.some((d) => d.id === s.id)),
        valid: L.ladderValid(draft),
        changed: ladderChanged || commandsChanged,
        /** 저장할 바꾸기 (바뀐 것만) */
        body: () => ({
            ...(ladderChanged ? { ladder: patch } : {}),
            ...(commandsChanged ? { commandLevels } : {}),
        }),
        /** 맞바꾸기 고르기를 켜고 끈다 */
        toggleChoosing: () => {
            setChoosing((c) => !c);
            setPicked(null);
        },
        pick: (id: number) => {
            if (picked === null || picked === id) return setPicked(picked === id ? null : id);
            flip();
            setSteps((s) => [...s, [picked, id]]);
            setPicked(null);
            setChoosing(false);
        },
        revert: () => {
            setDraft(saved);
            setSteps([]);
            setMoves({});
            setChoosing(false);
            setPicked(null);
        },
        // 기본값으로 초기화: 기본 사다리와 프리셋 레벨 (초안, 저장해야 반영. 저장하면 명령별 레벨을 지운다)
        resetToDefault: () => {
            flip();
            setDraft(L.ladderOf(null));
            setSteps([]);
            setChoosing(false);
            setPicked(null);
            setMoves(
                Object.fromEntries(items.filter((it) => !it.fixed).map((it) => [it.key as string, it.preset as number])),
            );
        },
    };
}
