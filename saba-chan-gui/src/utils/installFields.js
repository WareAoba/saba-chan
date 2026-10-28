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
