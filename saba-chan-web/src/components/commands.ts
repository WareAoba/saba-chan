import type { CommandField, CommandInput } from '@gui/types/daemon';
import { withCommandAliases } from '@gui/utils/commandRisk';
import { commandText } from '@gui/utils/serverView';
import { useEffect, useState } from 'react';
import type { Module, Server, TFn, WebApi } from '../types';

/** 데몬 카탈로그와 module.toml 이 합쳐진 명령 한 칸. 카탈로그는 source 를 더한다 */
export interface Command extends Partial<Omit<CommandField, 'inputs'>> {
    name: string;
    source?: string;
    inputs?: CommandInput[];
}

/**
 * 데몬 카탈로그(모듈 명령 + 켜진 익스텐션 명령)에 module.toml 정의를 입힌다 (GUI useInstanceCommands 와 같다).
 * 카탈로그는 risk 를 안 적은 명령도 'caution'으로 채우므로 모듈 값을 우선하고,
 * module.toml 에 없는 명령(익스텐션·동적 명령)은 데몬이 보낸 danger 만 믿는다
 */
export function mergeCommands(catalog: Command[] | null, own: CommandField[]): Command[] {
    if (!Array.isArray(catalog)) return own;
    const byName = Object.fromEntries(own.map((c) => [c.name, c]));
    return catalog.map((c) =>
        byName[c.name] && c.source !== 'extension'
            ? { ...c, ...byName[c.name], risk: byName[c.name].risk }
            : { ...c, risk: c.risk === 'danger' ? 'danger' : undefined },
    );
}

// 입력 창 칸 값 → 명령 인자, 빈 필수 칸 (앱 명령 입력 창과 같은 규칙)
export { commandArgs, missingInputs } from '@gui/utils/commandSyntax';

/** 명령과 입력에 모듈 locales 문구를 입힌다 (`mod_<모듈>:commands.<이름>`) */
export function localizeCommands(t: TFn, moduleName: string, list: Command[]) {
    return list.map((c) => {
        const ns = `mod_${moduleName}:commands.${c.name}.inputs`;
        return {
            ...c,
            ...commandText(t, moduleName, c),
            inputs: c.inputs?.map((input) => ({
                ...input,
                label: t(`${ns}.${input.name}.label`, { defaultValue: input.label || input.name }),
                placeholder: t(`${ns}.${input.name}.placeholder`, { defaultValue: input.placeholder || '' }),
            })),
        };
    });
}

/** 이 서버에서 쓸 수 있는 명령 (현지화까지) */
export function useCommands(
    api: WebApi,
    server: Pick<Server, 'id' | 'module'>,
    moduleData: Partial<Module> | null | undefined,
    t: TFn,
) {
    const [catalog, setCatalog] = useState<Command[] | null>(null);
    useEffect(() => {
        setCatalog(null);
        let alive = true;
        api.instanceCommands(server.id)
            .then((r) => {
                if (alive && Array.isArray(r?.commands)) setCatalog(r.commands);
            })
            .catch(() => {});
        return () => {
            alive = false;
        };
    }, [api, server.id]);
    // module.toml 별명(`[aliases.commands]`)을 입혀 둔다: 입력 줄이 별명도 이름으로 읽는다 (앱과 같다)
    const own = withCommandAliases(moduleData?.commands?.fields, moduleData?.command_aliases);
    return localizeCommands(t, server.module, mergeCommands(catalog, own));
}
