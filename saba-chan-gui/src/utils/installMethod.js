/**
 * 설치 방식 (M5): manual / download만 데몬에 내장이고, 그 밖의 이름은 같은 이름의 익스텐션이 맡는다.
 * 데몬은 익스텐션 방식으로 설치한 인스턴스의 extension_data에 `install_method_<방식>` = true와
 * `<방식>_<필드>`를 쓴다 (src/instance/mod.rs `apply_install_extension_data`).
 */
export const BUILTIN_INSTALL_METHODS = ['manual', 'download'];

/**
 * 이 인스턴스를 설치한 익스텐션 방식 이름 (그 방식을 처리하는 익스텐션 id), 없으면 null.
 * 모듈이 정한 방식을 먼저 보고, 모듈 정보가 없으면 extension_data의 표시로 찾는다.
 */
export function extensionInstallMethod(server, module) {
    const ext = server?.extension_data || {};
    const declared = module?.install?.method;
    if (declared) {
        return !BUILTIN_INSTALL_METHODS.includes(declared) && ext[`install_method_${declared}`] === true
            ? declared
            : null;
    }
    for (const [key, value] of Object.entries(ext)) {
        if (value !== true || !key.startsWith('install_method_')) continue;
        const method = key.slice('install_method_'.length);
        if (method && !BUILTIN_INSTALL_METHODS.includes(method)) return method;
    }
    return null;
}
