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
    const cmd = (commands || []).find((c) => c.name?.toLowerCase() === head);
    return cmd?.risk || 'safe';
}

export const needsConfirm = (commands, text) => riskOf(commands, text) === 'danger';
