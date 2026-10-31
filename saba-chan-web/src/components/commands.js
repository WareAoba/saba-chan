import { commandText } from '@gui/utils/serverView';
import { useEffect, useState } from 'react';

/**
 * 데몬 카탈로그(모듈 명령 + 켜진 익스텐션 명령)에 module.toml 정의를 입힌다 (GUI useInstanceCommands 와 같다).
 * 카탈로그는 risk 를 안 적은 명령도 'caution'으로 채우므로 모듈 값을 우선하고,
 * module.toml 에 없는 명령(익스텐션·동적 명령)은 데몬이 보낸 danger 만 믿는다
 */
export function mergeCommands(catalog, own) {
    if (!Array.isArray(catalog)) return own;
    const byName = Object.fromEntries(own.map((c) => [c.name, c]));
    return catalog.map((c) =>
        byName[c.name] && c.source !== 'extension'
            ? { ...c, ...byName[c.name], risk: byName[c.name].risk }
            : { ...c, risk: c.risk === 'danger' ? 'danger' : undefined },
    );
}

/** 입력 칸 값 → 명령 인자. 빈 칸은 빼고, 숫자 칸은 숫자로, 켜고 끄는 칸은 참거짓으로 (손대지 않은 필수 칸은 false) */
export function commandArgs(inputs = [], values = {}) {
    const args = {};
    for (const input of inputs) {
        const raw = values[input.name];
        if (input.type === 'boolean') {
            // 손대지 않은 칸은 꺼진 칸이다: 필수면 false 로 보낸다 (빼면 데몬이 missing_argument 로 거절한다)
            if (raw !== undefined && raw !== '') args[input.name] = raw === true || raw === 'true';
            else if (input.required) args[input.name] = false;
            continue;
        }
        const value = String(raw ?? '').trim();
        if (!value) continue;
        args[input.name] = input.type === 'number' && Number.isFinite(Number(value)) ? Number(value) : value;
    }
    return args;
}

/** 필수 칸이 비었는지 (기본값이 있으면 데몬이 채운다) */
export const missingInputs = (inputs = [], values = {}) =>
    inputs.filter(
        (i) => i.required && i.default == null && i.type !== 'boolean' && !String(values[i.name] ?? '').trim(),
    );

/** 명령과 입력에 모듈 locales 문구를 입힌다 (`mod_<모듈>:commands.<이름>`) */
export function localizeCommands(t, moduleName, list) {
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
export function useCommands(api, server, moduleData, t) {
    const [catalog, setCatalog] = useState(null);
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
    const own = moduleData?.commands?.fields ?? [];
    return localizeCommands(t, server.module, mergeCommands(catalog, own));
}
