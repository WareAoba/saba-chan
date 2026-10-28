/**
 * 새 서버 만들기 화면이 함께 쓰는 규칙 (GUI·웹 원격). CLI `server add`와 같게 맞춘다.
 */

/** 만들기 화면이 그리는 설치 칸 종류 (목록·여러 줄·비밀번호·파일 칸은 만든 뒤 설정 화면에서) */
const INSTALL_TYPES = ['select', 'boolean', 'text', 'number'];

/** 모듈이 만들 때 고르라고 한 설정 (`install = true`, 예: 마인크래프트 서버 종류) */
export const installFields = (module) =>
    (module?.settings?.fields || []).filter((f) => f.install && INSTALL_TYPES.includes(f.field_type || f.type || 'text'));

/** 보낼 설치 값: 비운 칸은 빼서 데몬이 모듈 기본값을 쓰게 한다 */
export const installSettings = (values) =>
    Object.fromEntries(Object.entries(values || {}).filter(([, v]) => v !== '' && v != null));

/** 만들 때 고를 수 있는 값: 사바쨩이 설치할 수 있는 값(`install_options`), 없으면 `options` 전부 */
export const installChoices = (field) => field?.install_options || field?.options || [];

/** 선택지 표시 이름: 모듈 locales `settings.<칸>.options.<값>`, 없으면 값 그대로 */
export const optionLabel = (t, moduleName, field, value) =>
    t(`mod_${moduleName}:settings.${field.name}.options.${value}`, { defaultValue: String(value) });

/** 같은 이름의 서버가 있는가 (대소문자·앞뒤 공백 무시, CLI `server add`와 같다) */
export const nameTaken = (servers, name) => {
    const wanted = String(name || '')
        .trim()
        .toLowerCase();
    return !!wanted && (servers || []).some((s) => String(s.name || '').toLowerCase() === wanted);
};

/** 기본 이름: 모듈 이름 그대로, 겹치면 `-2`부터 붙인다 */
export const suggestName = (moduleName, servers) => {
    for (let i = 1; ; i++) {
        const name = i === 1 ? moduleName : `${moduleName}-${i}`;
        if (!nameTaken(servers, name)) return name;
    }
};

/** 고를 서버 버전이 있는가: 내려받아 설치하는 모듈만 (`[install] method = "download"`, 데몬이 `list_versions`로 고른다) */
export const choosesVersion = (module) => module?.install?.method === 'download';

/** 고를 수 있는 서버 버전 (`moduleListVersions` 응답): 정식 릴리스만, 종류가 없는 항목은 그대로 (GUI·웹·CLI가 같다) */
export const releaseVersions = (res) =>
    (Array.isArray(res?.versions) ? res.versions : [])
        .filter((v) => !v?.type || v.type === 'release')
        .map((v) => String(v?.id ?? v));

/**
 * 새 서버 만들기 단계. 서버 종류·버전 단계는 고를 것이 있을 때만 (설치 칸, 추천 설정, 버전).
 * 다음 단계: 파일 위치 → 실행 방식 → 이름·포트 확인 → 확인(약관·익스텐션 칸)
 */
export const wizardSteps = (module) => [
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
export const createBody = ({ name, module, installValues, preset, version, folder, managed, port }) => ({
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
export const checkBlockers = (check) =>
    [
        check?.name?.taken && 'name_taken',
        check?.folder?.in_use_by && 'folder_in_use',
        check?.folder?.not_empty && 'folder_not_empty',
    ].filter(Boolean);
