import type { TFunction } from 'i18next';
import type { ApiBody, ModuleInfo, SettingField } from '../types/daemon';

/**
 * 새 서버 만들기 화면이 함께 쓰는 규칙 (GUI·웹 원격). CLI `server add`와 같게 맞춘다.
 */

/** 만들기 화면이 그리는 설치 칸 종류 (목록·여러 줄·비밀번호·파일 칸은 만든 뒤 설정 화면에서) */
const INSTALL_TYPES = ['select', 'boolean', 'text', 'number'];

/** 모듈이 만들 때 고르라고 한 설정 (`install = true`, 예: 마인크래프트 서버 종류) */
export const installFields = (module?: Pick<ModuleInfo, 'settings'> | null) =>
    (module?.settings?.fields || []).filter((f: SettingField & { type?: string }) => f.install && INSTALL_TYPES.includes(f.field_type || f.type || 'text'));

/** 보낼 설치 값: 비운 칸은 빼서 데몬이 모듈 기본값을 쓰게 한다 */
export const installSettings = (values?: Record<string, unknown> | null) =>
    Object.fromEntries(Object.entries(values || {}).filter(([, v]) => v !== '' && v != null));

/** 만들 때 고를 수 있는 값: 사바쨩이 설치할 수 있는 값(`install_options`), 없으면 `options` 전부 */
export const installChoices = (field?: Pick<SettingField, 'install_options' | 'options'> | null) => field?.install_options || field?.options || [];

/** 선택지 표시 이름: 모듈 locales `settings.<칸>.options.<값>`, 없으면 값 그대로 */
export const optionLabel = (t: TFunction, moduleName: string, field: { name: string }, value: unknown) =>
    t(`mod_${moduleName}:settings.${field.name}.options.${value}`, { defaultValue: String(value) });

/** 같은 이름의 서버가 있는가 (대소문자·앞뒤 공백 무시, CLI `server add`와 같다) */
export const nameTaken = (servers?: { name?: string }[] | null, name?: string | null) => {
    const wanted = String(name || '')
        .trim()
        .toLowerCase();
    return !!wanted && (servers || []).some((s) => String(s.name || '').toLowerCase() === wanted);
};

/** 기본 이름: 모듈 이름 그대로, 겹치면 `-2`부터 붙인다 */
/**
 * 쉼표가 든 서버 이름: 데몬이 만들기·이름 바꾸기를 400 `invalid_name`으로 막는다 (디스코드에서 부르는 말을 쉼표로 나눈다,
 * 사용자 11-06 #2. 띄어쓰기는 된다). 데몬 `chat::words::is_comma`와 같은 글자
 */
export const nameHasComma = (name?: string | null) => /[,，、､]/.test(String(name || ''));

export const suggestName = (moduleName: string, servers?: { name?: string }[] | null) => {
    for (let i = 1; ; i++) {
        const name = i === 1 ? moduleName : `${moduleName}-${i}`;
        if (!nameTaken(servers, name)) return name;
    }
};

/** 버전 목록을 물을 설정: 고른 추천 설정 위에 고친 설치 칸 (데몬 만들기가 합치는 순서와 같다) */
export const versionSettings = (module?: Pick<ModuleInfo, 'presets'> | null, installValues?: Record<string, unknown> | null, preset?: string) => ({
    ...(module?.presets?.find((p) => p.id === preset)?.settings || {}),
    ...installSettings(installValues),
});

/** 고를 서버 버전이 있는가: 내려받아 설치하는 모듈만 (`[install] method = "download"`, 데몬이 `list_versions`로 고른다) */
export const choosesVersion = (module?: Pick<ModuleInfo, 'install'> | null) => module?.install?.method === 'download';

/** 고를 수 있는 서버 버전 (`moduleListVersions` 응답): 정식 릴리스만, 종류가 없는 항목은 그대로 (GUI·웹·CLI가 같다) */
export const releaseVersions = (res?: ApiBody | null) =>
    (Array.isArray(res?.versions) ? res.versions : [])
        .filter((v) => !v?.type || v.type === 'release')
        .map((v) => String(v?.id ?? v));

/** 모듈 list_versions가 알려 준 받을 수 있는 최신 릴리스 (`latest.release`, 예전 모양 `latest_release`). 모르면 '' */
export const latestRelease = (res?: ApiBody | null): string => {
    const v = res?.latest?.release ?? res?.latest_release;
    return typeof v === 'string' ? v : '';
};

/**
 * 새 서버 만들기 단계. 서버 종류·버전 단계는 고를 것이 있을 때만 (설치 칸, 추천 설정, 버전).
 * 다음 단계: 파일 위치 → 실행 방식 → 이름·포트 확인 → 확인(약관·익스텐션 칸)
 */
export const wizardSteps = (module?: Pick<ModuleInfo, 'settings' | 'presets' | 'install'> | null) => [
    'select-game',
    ...(installFields(module).length || module?.presets?.length || choosesVersion(module) ? ['server'] : []),
    'location',
    'run',
    'name',
    'confirm',
];

/**
 * 만들기(`POST /api/instances`)와 그 확인(`/check`)에 보낼 몸체. 고르지 않은 것은 보내지 않아 데몬이 기본값을 쓴다:
 * 버전은 최신, 폴더는 사바쨩 폴더, 실행 방식은 사바쨩이 관리, 포트는 다른 서버와 겹치지 않게 데몬이 고른다
 */
export interface CreateBodyInput {
    name?: string;
    module: string;
    installValues?: Record<string, unknown> | null;
    preset?: string;
    version?: string;
    folder?: string;
    managed?: boolean;
    port?: number | string | null;
}

export const createBody = ({ name, module, installValues, preset, version, folder, managed, port }: CreateBodyInput) => ({
    name: String(name || '').trim(),
    module_name: module,
    module_settings: installSettings(installValues),
    ...(preset ? { preset } : {}),
    ...(version ? { version } : {}),
    ...(folder ? { working_dir: folder } : {}),
    ...(managed === false ? { managed_start: false } : {}),
    ...(port != null && port !== '' ? { port: Number(port) } : {}),
});

/** 확인 결과(`/api/instances/check`)에서 만들기를 막는 것: 같은 이름, 다른 서버가 쓰는 폴더, 파일이 든 폴더 */
export const checkBlockers = (check?: ApiBody | null) =>
    [
        check?.name?.taken && 'name_taken',
        check?.name?.invalid && 'name_invalid',
        check?.folder?.in_use_by && 'folder_in_use',
        check?.folder?.not_empty && 'folder_not_empty',
    ].filter(Boolean);
