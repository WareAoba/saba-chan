/**
 * 서버 한 개를 화면에 어떻게 보여줄지 정하는 순수 함수 모음.
 * 목록 줄과 상세 화면이 같은 규칙을 쓰도록 한곳에 둔다.
 */

/**
 * 폴링으로 받은 새 목록에서 내용이 그대로인 서버는 이전 객체를 쓰고, 전부 그대로면 이전 배열을 돌려준다.
 * 서버 목록은 몇 초마다 새로 오므로, 바뀐 게 없을 때 화면 전체를 다시 그리지 않게 한다.
 */
export function reuseUnchanged(prev, next) {
    const byName = new Map(prev.map((s) => [s.name, s]));
    let same = prev.length === next.length;
    const out = next.map((s, i) => {
        const old = byName.get(s.name);
        if (old && JSON.stringify(old) === JSON.stringify(s)) {
            if (prev[i] !== old) same = false;
            return old;
        }
        same = false;
        return s;
    });
    return same ? prev : out;
}

/**
 * 화면용 상태 하나로 접는다: running | stopped | starting | stopping | provisioning | missing.
 * 모듈 목록을 아직 못 받았으면(시작 직후) 모듈이 없다고 단정하지 않는다.
 */
export function viewStatus(server, moduleData, modulesLoaded = true) {
    if (!moduleData && modulesLoaded) return 'missing';
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

/**
 * 콘솔에 그대로 쳐도 되는 명령만 (콘솔 빠른 명령 칩).
 * 데몬 method "console"(예전 이름 "rcon")이고 console_template이 명령 이름으로 시작할 때만:
 * 칩은 이름을 입력칸에 채울 뿐이라, 템플릿이 다른 말로 바뀌는 명령이나 "module"/"rest" 명령은 콘솔에서 안 통한다.
 */
export function consoleQuickCommands(fields = []) {
    return (fields || []).filter((c) => {
        const method = c.method === 'rcon' || c.method === 'stdin' ? 'console' : c.method;
        if (method !== 'console') return false;
        const template = c.console_template ?? c.rcon_template;
        return !template || template.trim().split(/\s+/)[0].toLowerCase() === String(c.name).toLowerCase();
    });
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
