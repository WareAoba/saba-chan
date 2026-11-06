/** 인스턴스 낱말 초안 (화면 없는 순수 함수, GUI 서버 상세 디스코드 탭과 웹 디스코드 호출어가 같이 쓴다) */
import type { TFunction } from 'i18next';
import { commandText } from '../../utils/serverView';

/** GET /api/instance/:id/words 의 명령 하나 */
export interface InstanceWordsCommand {
    name: string;
    aliases?: string[];
    moduleAliases?: string[];
    label?: string;
    risk?: string;
    builtin?: boolean;
}

/** 데몬 내장 명령(이름표가 없다)의 화면 이름 (gui 네임스페이스 키) */
const BUILTIN_LABEL: Record<string, string> = {
    start: 'gui:shell.start',
    stop: 'gui:shell.stop',
    status: 'gui:discord.iw_cmd_status',
};

/** 명령의 화면 이름: 내장 명령은 앱 문구, 모듈 명령은 모듈 번역 (다른 화면과 같다, 사용자 11-06 #9) */
export function wordsCommandName(t: TFunction, moduleName: string, c: InstanceWordsCommand) {
    if (c.builtin && !c.label && BUILTIN_LABEL[c.name]) return t(BUILTIN_LABEL[c.name]);
    return commandText(t, moduleName, c).label;
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
