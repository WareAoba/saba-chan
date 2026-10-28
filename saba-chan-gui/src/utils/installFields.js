/**
 * 새 서버 만들기 화면이 함께 쓰는 규칙 (GUI·웹 원격). CLI `server add`와 같게 맞춘다.
 */

/** 모듈이 만들 때 고르라고 한 설정 (`install = true`, 예: 마인크래프트 서버 종류) */
export const installFields = (module) => (module?.settings?.fields || []).filter((f) => f.install);

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
