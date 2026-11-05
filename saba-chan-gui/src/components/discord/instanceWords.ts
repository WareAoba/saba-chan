/** 인스턴스 낱말 초안 (화면 없는 순수 함수, GUI 서버 상세 디스코드 탭과 웹 디스코드 호출어가 같이 쓴다) */

/** GET /api/instance/:id/words 의 명령 하나 */
export interface InstanceWordsCommand {
    name: string;
    aliases?: string[];
    moduleAliases?: string[];
    label?: string;
    risk?: string;
}

export interface InstanceWordsEntry {
    words?: string[];
    commands?: InstanceWordsCommand[];
}

export interface InstanceWordsDraft {
    words: string[];
    commands: Record<string, string[]>;
}

export interface ValidationDetail {
    field?: string;
    [key: string]: unknown;
}

const sameList = (a: string[] = [], b: string[] = []) => a.length === b.length && a.every((w, i) => w === b[i]);

/** 데몬 항목 → 고칠 수 있는 초안 `{words, commands: {명령: [낱말]}}` */
export function draftOf(inst: InstanceWordsEntry): InstanceWordsDraft {
    return {
        words: [...(inst.words || [])],
        commands: Object.fromEntries((inst.commands || []).map((c) => [c.name, [...(c.aliases || [])]])),
    };
}

export function isDraftDirty(
    draft: InstanceWordsDraft | null | undefined,
    inst: InstanceWordsEntry | null | undefined,
) {
    if (!draft || !inst) return false;
    if (!sameList(draft.words, inst.words || [])) return true;
    return (inst.commands || []).some((c) => !sameList(draft.commands[c.name], c.aliases || []));
}

/** 400 `validation_failed`의 details를 칸별로 나눈다: `words`, `commands.<명령>` */
export function errorsByField(details: ValidationDetail[] | null | undefined) {
    const out: Record<string, ValidationDetail[]> = {};
    for (const d of details || []) {
        const key = String(d?.field || '').replace(/\[\d+\]$/, '') || 'words';
        (out[key] ||= []).push(d);
    }
    return out;
}
