import type { TFunction } from 'i18next';
import type { CommandAliasInfo, ModuleInfo, ServerInfo } from '../types/daemon';
import { type CommandField, withCommandAliases } from './commandRisk';

/**
 * 서버 한 개를 화면에 어떻게 보여줄지 정하는 순수 함수 모음.
 * 목록 줄과 상세 화면이 같은 규칙을 쓰도록 한곳에 둔다.
 */

/**
 * 폴링으로 받은 새 목록에서 내용이 그대로인 서버는 이전 객체를 쓰고, 전부 그대로면 이전 배열을 돌려준다.
 * 서버 목록은 몇 초마다 새로 오므로, 바뀐 게 없을 때 화면 전체를 다시 그리지 않게 한다.
 */
export function reuseUnchanged<T extends { id: string }>(prev: T[], next: T[]) {
    const byId = new Map(prev.map((s) => [s.id, s]));
    let same = prev.length === next.length;
    const out = next.map((s, i) => {
        const old = byId.get(s.id);
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
export function viewStatus(server: Partial<ServerInfo>, moduleData?: unknown, modulesLoaded = true) {
    if (!moduleData && modulesLoaded) return 'missing';
    if (server.provisioning) return 'provisioning';
    // 켜는 중·끄는 중은 데몬이 `transition`에 싣는다 (어디서 시켰든)
    const status = server.transition ?? server.status;
    if (status === 'running' || status === 'starting' || status === 'stopping') return status;
    return 'stopped';
}

/**
 * 상태 → StatusChip 색 (tokens.css의 --ok/--stop/--warn/--info).
 * 켜는 중·끄는 중은 떠나온 상태의 색으로 깜빡인다 (사용자 11-11): 켜는 중 = 꺼짐 회색, 끄는 중 = 켜짐 초록. 노랑은 복구 중만
 */
export type StatusTone = 'ok' | 'stop' | 'warn' | 'info' | 'idle';

/** 깜빡이는(진행 중) 상태 — 설치·업데이트 중(provisioning)도 꺼짐에서 출발한 진행이다 */
export const isTransitional = (status: string | undefined) =>
    status === 'starting' || status === 'stopping' || status === 'provisioning';

export const STATUS_TONE: Record<string, StatusTone> = {
    running: 'ok',
    stopped: 'idle',
    starting: 'idle',
    stopping: 'ok',
    provisioning: 'idle',
    missing: 'warn',
};

/** 접속자 수 글자 ("3/20", 최대를 모르면 "3"). 데몬이 모르면(`players` 없음) null — 0으로 보이지 않는다 */
export function playersText(server: Partial<ServerInfo>) {
    const p = server.status === 'running' ? server.players : null;
    if (!p || !Number.isInteger(p.online)) return null;
    return Number.isInteger(p.max) ? `${p.online}/${p.max}` : `${p.online}`;
}

/** 켜기/끄기 버튼이 지금 무엇을 해야 하는지 */
export function primaryAction(status: string) {
    if (status === 'running') return 'stop';
    if (status === 'stopped') return 'start';
    return 'busy';
}

/** 메모리 상한 칸의 상태 (데몬이 싣는 memory_limit_supported 로 판단한다. 다른 PC의 데몬일 수 있어 앱 OS로 짐작하지 않는다):
 * 'editable' 걸 수 있음(싣지 않는 예전 데몬 포함), 'locked' 걸 수 없음, 'stale' 걸 수 없는데 예전에 저장된 값이 남음 */
export function memoryLimitState(server: Partial<ServerInfo>): 'editable' | 'locked' | 'stale' {
    if (server.memory_limit_supported !== false) return 'editable';
    return server.memory_limit_mb == null ? 'locked' : 'stale';
}

/**
 * 데몬이 이 서버를 managed(saba-runner가 stdin/stdout을 쥔)로 켜는가.
 * 데몬이 알려 주는 `managed`가 있으면 그것을, 없으면 데몬 규칙(Supervisor::uses_managed_start)을 따른다:
 * 인스턴스 설정 `managed_start`가 우선, 없으면 managed가 기본.
 */
export function isManaged(server?: Partial<ServerInfo> | null) {
    if (typeof server?.managed === 'boolean') return server.managed;
    const v = server?.module_settings?.managed_start;
    if (v === false || v === 'false') return false;
    return true;
}

/**
 * 이 서버에 콘솔과 명령 목록 중 무엇을 보여줄지.
 * managed면 러너가 출력을 받으므로 콘솔은 늘 보인다. 입력(`input`)은 모듈이 stdin을 받을 때만이고,
 * REST 전용 서버는 읽기 전용 콘솔에 명령 탭을 함께 둔다.
 */
export function interactionOf(server: Partial<ServerInfo>, moduleData?: Partial<ModuleInfo> | null) {
    const supported = moduleData?.protocols?.supported || [];
    const hasCommands = (moduleData?.commands?.fields || []).length > 0;
    const hasConsole = isManaged(server);
    const hasInput = hasConsole && supported.includes('stdin');
    return { console: hasConsole, input: hasInput, commands: hasCommands || !hasInput };
}

/**
 * 서버를 처음 열 때의 탭: 콘솔이 있으면 콘솔(입력을 못 받는 REST 전용 서버도 지난 출력을 본다),
 * 없으면 켜져 있을 때 명령, 꺼져 있을 때 정보
 */
export function defaultServerTab(interaction: ReturnType<typeof interactionOf>, status?: string) {
    if (interaction.console) return 'console';
    return status === 'running' && interaction.commands ? 'commands' : 'info';
}

/**
 * 콘솔에 그대로 쳐도 되는 명령만 (콘솔 빠른 명령 칩).
 * 데몬 method "console"이고 console_template이 명령 이름으로 시작할 때만 (예전 이름 `rcon`·`rcon_template`은 데몬이 바꿔 준다):
 * 칩은 이름을 입력칸에 채울 뿐이라, 템플릿이 다른 말로 바뀌는 명령이나 "module"/"rest" 명령은 콘솔에서 안 통한다.
 */
export function consoleQuickCommands(fields: CommandField[] = [], commandAliases: Record<string, CommandAliasInfo> | null = null) {
    return withCommandAliases(fields, commandAliases).filter((c) => {
        if (c.method !== 'console') return false;
        const template = c.console_template;
        return !template || template.trim().split(/\s+/)[0].toLowerCase() === String(c.name).toLowerCase();
    });
}

/** 명령 표시 문구: 모듈 locales(`mod_<모듈>:commands.<이름>`)가 앞서고, 없으면 모듈이 준 문구, 그다음 명령 이름 */
export function commandText(t: TFunction, moduleName: string, cmd: { name: string; label?: string | null; description?: string | null }) {
    const ns = `mod_${moduleName}:commands.${cmd.name}`;
    return {
        label: t(`${ns}.label`, { defaultValue: cmd.label || cmd.name }),
        description: t(`${ns}.description`, { defaultValue: cmd.description || '' }),
    };
}

/** 명령 입력 칸 이름: 모듈 locales(`mod_<모듈>:commands.<명령>.inputs.<칸>.label`)가 앞서고, 없으면 모듈이 준 이름, 그다음 칸 이름 */
export function commandInputLabel(t: TFunction, moduleName: string, commandName: string, input: { name: string; label?: string | null }) {
    return t(`mod_${moduleName}:commands.${commandName}.inputs.${input.name}.label`, { defaultValue: input.label || input.name });
}

/** 이름 첫 글자로 만든 아이콘 대체 글자 */
export function initials(name = '') {
    const trimmed = String(name).trim();
    return trimmed ? Array.from(trimmed)[0].toUpperCase() : '?';
}

/** 모듈이 정한 프로토콜 표시 이름 */
export function protocolLabel(server: Partial<ServerInfo>) {
    // `auto`·예전 `rcon`은 데몬이 모듈 프로토콜로 바꿔 준다 (`ModuleMetadata::protocol_mode_for`)
    return server.protocol_mode?.toUpperCase() || 'AUTO';
}

/** 빌드를 받은(올라온) 날짜 (유닉스 초): 올해면 월·일, 다른 해면 연도까지. 모르면 빈 글자 */
export function buildDate(time: unknown, lang: string) {
    if (typeof time !== 'number' || !(time > 0)) return '';
    const at = new Date(time * 1000);
    const year = at.getFullYear() === new Date().getFullYear() ? undefined : 'numeric';
    return at.toLocaleDateString(lang, { year, month: 'long', day: 'numeric' });
}

/** 화면에 보일 게임 이름: 모듈 번역(display_name), 없으면 모듈의 game_name, 그것도 없으면 모듈 id */
export function gameNameOf(t: TFunction, moduleName: string, moduleData?: Partial<ModuleInfo> | null) {
    return t(`mod_${moduleName}:module.display_name`, { defaultValue: moduleData?.game_name || moduleName });
}

/** 브랜치 서버의 상위 서버 (데몬 `branch_of`). 목록에 없으면 독립 서버로 본다 */
export function branchParentOf<T extends ServerInfo>(server: T, servers: T[]): T | undefined {
    return server.branch_of ? servers.find((s) => s.id === server.branch_of && !s.branch_of) : undefined;
}

/** 브랜치 서버를 상위 서버 바로 아래로 모은다 (나머지 순서는 그대로). 옮길 것이 없으면 받은 배열을 돌려준다 */
export function branchesUnderParents<T extends ServerInfo>(servers: T[]): T[] {
    if (!servers.some((s) => s.branch_of)) return servers;
    const top = servers.filter((s) => !branchParentOf(s, servers));
    const grouped = top.flatMap((parent) => [parent, ...servers.filter((s) => branchParentOf(s, servers) === parent)]);
    return grouped.every((s, i) => s === servers[i]) ? servers : grouped;
}
