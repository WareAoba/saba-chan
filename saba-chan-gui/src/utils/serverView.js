/**
 * 서버 한 개를 화면에 어떻게 보여줄지 정하는 순수 함수 모음.
 * 목록 줄과 상세 화면이 같은 규칙을 쓰도록 한곳에 둔다.
 */

/** 화면용 상태 하나로 접는다: running | stopped | starting | stopping | provisioning | missing */
export function viewStatus(server, moduleData) {
    if (!moduleData) return 'missing';
    if (server.provisioning) return 'provisioning';
    if (server.status === 'running' || server.status === 'starting' || server.status === 'stopping') {
        return server.status;
    }
    return 'stopped';
}

/** 상태 → 상태 칩 색 이름 (tokens.css의 --ok/--stop/--warn/--info) */
export const STATUS_TONE = {
    running: 'ok',
    stopped: 'stop',
    starting: 'warn',
    stopping: 'warn',
    provisioning: 'info',
    missing: 'warn',
};

/** 켜기/끄기 버튼이 지금 무엇을 해야 하는지 */
export function primaryAction(status) {
    if (status === 'running') return 'stop';
    if (status === 'stopped') return 'start';
    return 'busy';
}

/**
 * 데몬이 이 서버를 managed(saba-runner가 stdin/stdout을 쥔)로 켜는가.
 * 데몬이 알려 주는 `managed`가 있으면 그것을, 없으면 데몬 규칙(Supervisor::uses_managed_start)을 따른다:
 * 인스턴스 설정 `managed_start`가 우선, 없으면 managed가 기본.
 */
export function isManaged(server) {
    if (typeof server?.managed === 'boolean') return server.managed;
    const v = server?.module_settings?.managed_start;
    if (v === false || v === 'false') return false;
    return true;
}

/** 이 서버에 콘솔(stdin)과 명령 목록 중 무엇을 보여줄지 */
export function interactionOf(server, moduleData) {
    const supported = moduleData?.protocols?.supported || [];
    const hasCommands = (moduleData?.commands?.fields || []).length > 0;
    const hasConsole = isManaged(server) && supported.includes('stdin');
    return { console: hasConsole, commands: hasCommands || !hasConsole };
}

/** 이름 첫 글자로 만든 아이콘 대체 글자 */
export function initials(name = '') {
    const trimmed = String(name).trim();
    return trimmed ? Array.from(trimmed)[0].toUpperCase() : '?';
}

/** 모듈이 정한 프로토콜 표시 이름 */
export function protocolLabel(server, moduleData) {
    const proto = server.protocol_mode;
    // 'rcon'은 데몬에서 빠졌다: 예전 값이면 모듈 기본으로 보여 준다
    if (proto === 'auto' || proto === 'rest' || proto === 'rcon') {
        const supported = moduleData?.protocols?.supported || [];
        if (proto === 'rest' && supported.includes('rest')) return 'REST';
        if (moduleData?.protocols?.default) return moduleData.protocols.default.toUpperCase();
        if (supported.length > 0) return supported[0].toUpperCase();
    }
    return proto?.toUpperCase() || 'AUTO';
}
