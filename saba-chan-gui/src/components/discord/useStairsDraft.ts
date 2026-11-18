import { useEffect, useMemo, useRef, useState } from 'react';
import { useChipFlip } from './flip';
import * as L from './levels';
import { usePointerDrag } from './pointerDrag';

/**
 * 명령 계단 초안 (앱 `CommandStairs`·웹 `PermissionStairs`가 같이 쓴다, 그리는 모양은 화면마다 다르다).
 * 칩을 끌어 다른 레벨 칸에 놓으면 초안으로 옮기고, 저장 막대에서 저장한다.
 * 데몬 값을 다시 읽으면 저장된 레벨과 같아진 옮김을 내려놓는다(저장 뒤에는 초안이 빈다). 되돌리기는 초안을 비운다. 칩은 저장된 값이 바뀔 때도, 초안에서 옮길 때도 미끄러진다.
 * 놓은 칩을 옮기는 일은 그 화면이 보이는 칩을 알고 나서 `dropRef`에 채운다
 */
export function useStairsDraft<T extends HTMLElement>(board: L.CommandBoard | null | undefined, busy: boolean) {
    const dropRef = useRef<(id: string, level: number) => void>(() => {});
    const chipDrag = usePointerDrag((id: string, to: string) => dropRef.current(id, Number(to)));
    // 명령 키 → 옮긴 레벨
    const [moves, setMoves] = useState<Record<string, number>>({});
    // 데몬 값을 다시 읽으면 저장된 레벨과 같아진 옮김만 내려놓는다 (초안 저장 뒤에는 모두 같아져 비고,
    // 되묻기 단계·프리셋 같은 즉시 저장 뒤의 다시 읽기는 옮겨 둔 칩을 지우지 않는다). 계단을 못 읽으면 비운다
    useEffect(
        () =>
            setMoves((m) => {
                const saved = new Map(
                    (board?.items || []).filter((it) => !it.fixed).map((it) => [it.key as string, it.level]),
                );
                const kept = Object.entries(m).filter(([key, level]) => saved.has(key) && saved.get(key) !== level);
                return kept.length === Object.keys(m).length ? m : Object.fromEntries(kept);
            }),
        [board],
    );
    const ref = useRef<T>(null);
    // biome-ignore lint/correctness/useExhaustiveDependencies: board·moves 가 바뀔 때마다 새 표지
    const flipToken = useMemo(() => ({}), [board, moves]);
    useChipFlip(ref, flipToken);

    const items = board?.items || [];
    const draftLevels = items.map((it) =>
        !it.fixed && moves[it.key as string] !== undefined ? moves[it.key as string] : it.level,
    );
    const patch = L.boardPatch(board, draftLevels);
    // 노란 테두리는 저장하지 않은 변경: 저장된 레벨과 초안 레벨이 다른 명령 (제자리로 돌려 놓으면 사라진다)
    const savedLevel = new Map(items.map((it) => [it.key as string, it.level]));
    const draftTo = (list: L.CommandItem[], level: (it: L.CommandItem) => number) =>
        setMoves((m) => ({
            ...m,
            ...Object.fromEntries(list.filter((it) => !it.fixed).map((it) => [it.key as string, level(it)])),
        }));
    return {
        ref,
        dropRef,
        chipDrag,
        dragging: chipDrag.dragging,
        over: chipDrag.over === null ? null : Number(chipDrag.over),
        setMoves,
        shownBoard: L.draftBoard(board, draftLevels),
        patch,
        dirty: Object.keys(patch).length > 0,
        unsaved: (c: L.StairChip) => c.items.some((it) => savedLevel.get(it.key as string) !== it.level),
        draftTo,
        move: (chip: L.StairChip, level: number) => {
            // 대표 레벨이 같아도 다른 레벨인 인스턴스가 있으면 맞춘다
            if (busy || chip.fixed || level === L.BANNED || chip.items.every((i) => i.fixed || i.level === level))
                return;
            draftTo(chip.items, () => level);
        },
    };
}
