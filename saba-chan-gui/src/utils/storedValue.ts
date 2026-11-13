import type { ApiBody, Json } from '../types/daemon';

/** 0.1.14 정식(beta.2 전) 데몬용: 데몬이 인스턴스 최상위에 저장하는 공통 칸 (updater `constants::INSTANCE_FIELDS` 중 설정 칸이 되는 것) */
const TOP_LEVEL_FIELDS = new Set([
    'port',
    'rest_host',
    'rest_port',
    'rest_username',
    'rest_password',
    'executable_path',
    'working_dir',
    'protocol_mode',
    'server_version',
    'extra_args',
]);

/**
 * 설정 칸의 저장된 값 (없으면 undefined). 0.1.14-beta.2~ 데몬은 칸마다 진짜 값을 `stored`에 실어 준다(서버 목록·GET /api/instance/:id):
 * 공통 칸은 인스턴스 칸, 나머지는 module_settings. `stored`가 없는 0.1.14 데몬이면 같은 규칙으로 고른다
 * (원격 데몬 최소 버전이 beta.2를 넘을 때 이 대체 규칙을 지운다, `MIN_REMOTE_DAEMON` 참고). GUI 설정 스토어와 웹 설정 탭이 함께 쓴다
 */
export function storedValue(server: ApiBody | null | undefined, name: string): Json | undefined {
    if (server?.stored && typeof server.stored === 'object') return server.stored[name] ?? undefined;
    const top = server?.[name] ?? undefined;
    const cached = server?.module_settings?.[name] ?? undefined;
    return TOP_LEVEL_FIELDS.has(name) ? (top ?? cached) : (cached ?? top);
}
