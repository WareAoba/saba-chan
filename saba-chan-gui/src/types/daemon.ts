/**
 * 데몬 HTTP 응답 모양. 기준은 데몬 Rust 구조체다 (이름 옆 괄호가 그 구조체).
 * 화면 여러 곳이 쓰는 응답(서버·모듈·설정)은 필드까지 적고, 나머지는 ApiBody로 받는다.
 */

/** JSON 값 하나 */
export type Json = string | number | boolean | null | Json[] | { [key: string]: Json };
export type JsonObject = { [key: string]: Json };

/**
 * 데몬·브리지가 돌려주는 본문의 공통 칸. 실패면 error(사람이 읽는 문구)와 error_code(기계가 읽는 이유)가 온다.
 * 아직 모양을 적지 않은 응답은 이 타입으로 받고, 나머지 칸은 열어 둔다.
 */
export interface ApiErrorFields {
    ok?: boolean;
    success?: boolean;
    error?: string;
    error_code?: string;
    message?: string;
}

// 모양을 다 적지 않은 응답: 공통 칸 밖은 쓰는 쪽이 좁혀 읽는다
// biome-ignore lint/suspicious/noExplicitAny: 아직 타입을 적지 않은 데몬 응답의 나머지 칸
export type ApiBody = ApiErrorFields & { [key: string]: any };

/** 성공하면 T의 칸이 오고, 실패하면 공통 오류 칸만 온다: 쓰는 쪽은 칸이 없을 수 있다고 보고 읽는다 */
export type ApiResult<T> = Partial<T> & ApiErrorFields;

// ── 서버 (ipc::ServerInfo) ──

/** 전용 멤버 명단 한 줄 (데몬 controllers::Controller): 게임 닉네임과 이은 디스코드 사용자 ID */
export interface Controller {
    name: string;
    discord_id?: string;
}

/** 게임 채팅 전달 (supervisor::game_chat::Mode) */
export type GameChatMode = 'off' | 'all' | 'keyword';

/** ipc::PortConflictInfo */
export interface PortConflictInfo {
    port: number;
    port_type: string;
    conflict_name: string;
    conflict_id: string;
    conflict_port_type: string;
}

/** supervisor::players::PlayerCount */
export interface PlayerCount {
    online: number;
    max?: number;
}

/** GET /api/servers 항목 (ipc::ServerInfo) */
export interface ServerInfo {
    id: string;
    name: string;
    module: string;
    /** running · stopped · starting · stopping · provisioning · … (데몬이 정한다) */
    status: string;
    pid: number | null;
    start_time: number | null;
    executable_path: string | null;
    working_dir?: string | null;
    port: number | null;
    rest_host: string | null;
    rest_port: number | null;
    rest_username: string | null;
    rest_password: string | null;
    protocol_mode: string;
    module_settings: Record<string, Json>;
    server_version: string | null;
    /** 설치된 빌드: 설치 방식 익스텐션의 업데이트 확인이 알려 준 번호와 받은 때(유닉스 초). 데몬이 적어 둔 값 */
    server_build?: { id: string; time?: number } | null;
    /** API(CLI/Discord)로 마지막 시작·정지를 요청한 때 (epoch ms) */
    last_api_action?: number;
    provisioning: boolean;
    managed: boolean;
    extension_status?: Record<string, Json>;
    extension_data: Record<string, Json>;
    port_conflicts?: PortConflictInfo[];
    extra_args: string[];
    revision: string;
    memory_limit_mb: number | null;
    /** 게임 서버 자동 업데이트 (설치 방식 익스텐션으로 설치한 서버만, PC에서만 바꾼다) */
    auto_update?: boolean;
    game_chat: GameChatMode;
    game_chat_keyword: string;
    game_chat_supported: boolean;
    /** 사용 모드: 서버 공용(기본, 길드 레벨 권한) 또는 전용 멤버(명단에 있는 사람만) */
    access?: 'shared' | 'members';
    /** 전용 멤버 명단 (전용 멤버 모드에서 모든 조작·명령을 하는 사람) */
    controllers?: Controller[];
    /** 전용 멤버 모드의 자동 강퇴: 명단에 없는 닉네임은 접속하면 내보낸다 */
    controllers_only?: boolean;
    /** 서버 공용 모드의 자동 강퇴: 이 닉네임이 접속하면 내보낸다 */
    kick_players?: string[];
    /** 자동 강퇴를 쓸 수 있는지: 모듈이 접속 줄과 내보내기 명령을 알려 주고 데몬이 콘솔을 읽을 때만 참 */
    controllers_only_supported?: boolean;
    /** 접속자 없을 때 자동 정지를 쓸 수 있는지: 데몬이 접속자 수를 알 길이 있을 때만 참 */
    idle_stop_supported?: boolean;
    /** 꺼져 있고 마지막 정지가 접속자 없음 자동 정지였을 때: 정지한 시각(Unix 초)과 비어 있던 분 */
    idle_stopped?: { at: number; minutes: number };
    slots?: JsonObject;
    players?: PlayerCount;
    unresponsive?: string;
    /** `unresponsive`가 있을 때 답하지 않은 분 */
    unresponsive_minutes?: number;
    linked_folder?: string;
    restorable_settings: boolean;
}

/** 포트 충돌로 데몬이 강제로 끈 서버 알림 */
export interface PortConflictStopEvent {
    id?: number;
    stopped_name: string;
    stopped_id?: string;
    existing_name: string;
    port: number;
    [key: string]: Json | undefined;
}

/** GET /api/servers */
export interface ServerListResponse {
    servers: ServerInfo[];
    port_conflict_stops?: PortConflictStopEvent[];
}

// ── 모듈 (ipc::ExtensionInfo, supervisor::module_loader) ──

/** module_loader::VisibleWhen */
export interface VisibleWhen {
    field: string;
    equals?: Json;
    in?: Json[];
}

/** [[settings.fields]] (module_loader::SettingField) */
export interface SettingField {
    name: string;
    /** text · number · password · file · select · boolean · list · multiline */
    field_type: string;
    label: string;
    description: string | null;
    required: boolean | null;
    default: Json | null;
    min: number | null;
    max: number | null;
    step: number | null;
    options: string[] | null;
    /** basic(기본) · advanced · saba-chan … */
    group: string | null;
    port: boolean;
    public?: string;
    install?: boolean;
    install_options?: string[];
    arg?: string[];
    /** memory_size · port · integer · path */
    format?: string;
    pattern?: string;
    min_items?: number;
    max_items?: number;
    max_length?: number;
    restart_required?: boolean;
    visible_when?: VisibleWhen;
    file_key?: string;
}

/** module_loader::CommandInput */
export interface CommandInput {
    name: string;
    label: string | null;
    type: string | null;
    required: boolean | null;
    placeholder: string | null;
    default: Json | null;
    min?: number;
    max?: number;
    options?: string[];
}

/** [[commands.fields]] (module_loader::CommandField) */
export interface CommandField {
    name: string;
    label: string;
    description: string | null;
    method: string | null;
    http_method: string | null;
    endpoint_template: string | null;
    console_template: string | null;
    risk?: string;
    inputs: CommandInput[];
    /** 빠지면 true */
    remote?: boolean;
    reply_pattern?: string;
}

/** module_loader::ModuleCommands */
export interface ModuleCommands {
    fields: CommandField[];
    hook?: boolean;
    failure_patterns?: string[];
}

/** module_loader::HighlightRule */
export interface HighlightRule {
    name: string;
    pattern: string;
    color: string;
    background?: string;
    bold: boolean;
    italic: boolean;
    underline: boolean;
    priority: number;
}

/** module_loader::CommandAliasInfo */
export interface CommandAliasInfo {
    aliases: string[];
    description: string | null;
}

/** module_loader::ModulePreset */
export interface ModulePreset {
    id: string;
    label?: string;
    description?: string;
    settings: JsonObject;
}

/** ipc::UnmetDependency */
export interface UnmetDependency {
    component: string;
    required: string;
    installed: string | null;
}

/** [[agreements]] (module_loader::ModuleAgreement) */
export interface ModuleAgreement {
    id: string;
    url: string;
}

/** GET /api/modules 항목 (ipc::ExtensionInfo) */
export interface ModuleInfo {
    name: string;
    version: string;
    /** verified · modified · unverified */
    trust: string;
    description: string | null;
    schema_version: number;
    display_name?: string;
    game_name?: string;
    author?: string;
    warnings?: string[];
    unmet_dependencies?: UnmetDependency[];
    platforms?: string[];
    architectures?: string[];
    platform_supported: boolean;
    /** os · arch */
    unsupported_reason?: string;
    path: string;
    executable_path: string | null;
    server_executable?: string;
    icon: string | null;
    interaction_mode: string | null;
    protocols: { supported: string[]; default: string | null } | null;
    settings: { fields: SettingField[] } | null;
    commands: ModuleCommands | null;
    syntax_highlight: { rules: HighlightRule[] } | null;
    dir_signatures?: string[];
    install?: JsonObject;
    module_aliases?: string[];
    command_aliases?: Record<string, CommandAliasInfo>;
    presets?: ModulePreset[];
    queries?: JsonObject[];
    agreements?: ModuleAgreement[];
    /** 모듈이 밝힌 lifecycle 함수. null이면 모른다 (아무 기능도 숨기지 않는다) */
    functions: string[] | null;
}

/** module_loader::ModuleLoadError */
export interface ModuleLoadError {
    path: string;
    error: string;
    module?: string;
    error_code?: string;
    /** 못 맞춘 의존성 (`"saba-core >=0.3.0 (installed 0.1.1)"` 꼴) */
    missing_dependencies?: string[];
    /** module_schema_invalid일 때 무엇이 틀렸는지 */
    problems?: string[];
    /** module_schema_unsupported일 때 모듈이 요구한 스키마 버전 */
    required?: number;
}

/** GET /api/modules (ipc::ExtensionListResponse) */
export interface ModuleListResponse {
    modules: ModuleInfo[];
    failed?: ModuleLoadError[];
}

// ── 익스텐션(플러그인) ──

/** extension::GuiManifest */
export interface ExtensionGuiManifest {
    bundle: string | null;
    styles: string | null;
    builtin: boolean | null;
    /** slot id → 표시 정보 (탭 이름·아이콘·설명, 이름과 설명은 익스텐션 i18n 키나 글자). 옛 모양은 컴포넌트 이름 문자열 */
    slots: Record<string, string | { label?: string; icon?: string; description?: string }>;
}

/** extension::FieldDef */
export interface ExtensionFieldDef {
    type: string;
    default: Json | null;
    optional: boolean | null;
}

/** GET /api/extensions 항목 (extension::ExtensionListItem) */
export interface ExtensionListItem {
    id: string;
    name: string;
    version: string;
    description: string;
    author: string;
    enabled: boolean;
    hooks: string[];
    dependencies: Record<string, string>;
    gui: ExtensionGuiManifest | null;
    cli: JsonObject | null;
    slots: JsonObject;
    instance_fields: Record<string, ExtensionFieldDef>;
    config_fields: Record<string, ExtensionFieldDef>;
    has_icon: boolean;
    schema_version: number;
    min_app_version?: string;
    incompatible?: string;
    warnings?: string[];
    /** verified · modified · unverified */
    trust: string;
    gui_allowed?: boolean;
}

/** GET /api/extensions */
export interface ExtensionListResponse {
    extensions: ExtensionListItem[];
    failed?: ModuleLoadError[];
}

// ── 릴레이 에이전트 ──

/** window.api.discordBotStatus: 데몬 릴레이 에이전트 상태. 릴레이가 이 앱 버전이나 상태 보고를 받지 않으면 객체로 온다 */
export type AgentStatus =
    | 'running'
    | 'stopped'
    | 'replaced'
    | 'invalid_relay_url'
    | 'unknown'
    | { status: 'too_old'; running: boolean; minVersion: string | null }
    | { status: 'rejected'; running: boolean; reason: HeartbeatReject };

/** 릴레이가 이 PC의 상태 보고(heartbeat)를 거절한 이유 (데몬 `heartbeatRejected`): 어느 칸이, 상한이면 상한과 보낸 크기 */
export interface HeartbeatReject {
    code: string;
    field: string;
    max?: number;
    actual?: number;
}

// ── GUI 설정 (config_store::GuiSettings, camelCase) ──

/** config_store::SoundSettings */
export interface SoundSettings {
    enabled: boolean;
    serverStart: boolean;
    serverStop: boolean;
    crash: boolean;
    install: boolean;
    update: boolean;
    startup: boolean;
}

/** config_store::ConsoleFonts: 글자 종류마다 글꼴 id */
export interface ConsoleFonts {
    latin: string;
    ko: string;
    ja: string;
    zh: string;
}

/** GET /api/config/gui (config_store::GuiSettings). 데몬이 없으면 셸의 로컬 사본이라 칸이 빠질 수 있다 */
export type SystemMonitorMode = 'off' | 'docked' | 'floating';

export interface GuiSettings {
    autoRefresh: boolean;
    refreshInterval: number;
    ipcPort: number;
    consoleBufferSize: number;
    autoGeneratePasswords: boolean;
    portConflictCheck: boolean;
    autoPortCheck: boolean;
    osNotifications: boolean;
    sounds: SoundSettings;
    /** ask · tray · quit */
    closeAction: string;
    /** short · normal · long */
    toastDuration: string;
    newServerBackupKeep: number;
    newServerAutoRestart: boolean;
    auditKeepDays: number;
    /** off · 24h · 12h */
    consoleTimestamps: string;
    consoleTimestampDate: boolean;
    /** 콘솔 긴 줄 접기 */
    consoleWrap: boolean;
    consoleGlass: boolean;
    /** 맨 아래를 볼 때 새 줄 따라가기 */
    consoleAutoScroll: boolean;
    /** auto · light · dark. 예전 버전은 저장하지 않았다 */
    theme?: string;
    accentColor: string;
    accentSecondary: string;
    useGradient: boolean;
    fontScale: number;
    enableTransitions: boolean;
    consoleSyntaxHighlight: boolean;
    consoleBgColor: string;
    consoleTextColor: string;
    sidebarCompact: boolean;
    windowOpacity: number;
    /** 창 바탕 반투명 (utils/themeManager WindowBackdropConfig) */
    windowBackdrop?: Partial<import('../utils/themeManager').WindowBackdropConfig>;
    consoleFontScale: number;
    consoleTokenColors: Record<string, string>;
    consoleFonts: ConsoleFonts;
    systemMonitorExpanded: boolean;
    /** 고급: 포트·서버별 네트워크 (기본 꺼짐, 데몬 `system_monitor_net_trace`) */
    systemMonitorNetTrace: boolean;
    monitorItems?: string[] | null;
    /** 시스템 모니터 표시 방식: off · docked(기본) · floating */
    systemMonitorMode?: SystemMonitorMode;
    /** 떠 있는 시스템 모니터 자리 (components/monitor/floatPosition.ts) */
    systemMonitorFloat?: import('../components/monitor/floatPosition').MonitorFloat | null;
    /** ✕로 닫은 가이드 id */
    closedGuides?: string[];
    windowBounds?: { width: number; height: number; x?: number; y?: number };
    language?: string;
    /** 앱을 열면 먼저 볼 원격 데몬 hostId */
    mainDaemon?: string;
    /** switch · unified */
    daemonView?: string;
}

// ── 데몬 HTTP 요청 (Rust `daemon_request`) ──

export type HttpMethod = 'get' | 'post' | 'put' | 'patch' | 'delete';

export interface RequestOptions {
    timeout?: number;
    responseType?: 'text' | 'json' | 'base64';
    /** 이 요청만 그 데몬에 보낸다 (원격 데몬 hostId, 이 PC는 'local') */
    target?: string;
}

/** axios 응답처럼 */
export interface DaemonResponse<T = ApiBody> {
    status: number;
    data: T;
}

/** axios 오류처럼: HTTP 오류면 response, 연결 실패면 code */
export interface DaemonError extends Error {
    code?: string;
    response?: DaemonResponse;
}
