/**
 * 모듈 명령의 위험도 (module.toml [[commands.fields]] risk = safe | caution | danger).
 * danger 명령은 실행 전에 한 번 더 확인한다. 위험도 표시는 데몬이 준 값만 쓴다.
 */
export function riskOf(commands, text) {
    const head = String(text || '')
        .trim()
        .split(/\s+/)[0]
        ?.toLowerCase();
    if (!head) return 'safe';
    // 데몬은 module.toml 별명도 원래 명령으로 풀어 실행하므로 별명으로 친 것도 같은 위험도다
    const cmd = (commands || []).find(
        (c) => c.name?.toLowerCase() === head || (c.aliases || []).some((a) => String(a).toLowerCase() === head),
    );
    return cmd?.risk || 'safe';
}

/** 모듈 목록의 명령(fields)에 `command_aliases`의 별명을 붙인다 (이미 있으면 그대로) */
export function withCommandAliases(fields, commandAliases) {
    return (fields || []).map((c) =>
        c.aliases || !commandAliases?.[c.name]?.aliases ? c : { ...c, aliases: commandAliases[c.name].aliases },
    );
}

export const needsConfirm = (commands, text) => riskOf(commands, text) === 'danger';
