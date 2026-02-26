/**
 * 설정값 타입 검증 및 포트/별명 충돌 검사 유틸리티
 *
 * - validateSettingValue: module.toml SettingField 스키마 기반 개별 필드 검증
 * - validateAllSettings: 전체 설정 필드 일괄 검증
 * - checkPortConflicts: 모든 인스턴스 간 포트 충돌 검사
 * - checkAliasConflicts: 모듈 별명 충돌 검사
 */

/**
 * 개별 설정 필드의 값을 타입·범위·필수여부 기준으로 검증합니다.
 *
 * @param {Object} field - module.toml의 settings.fields 항목
 * @param {*} value - 사용자 입력값
 * @returns {{ valid: boolean, error?: string, errorType?: string }}
 */
function validateSettingValue(field, value) {
    const name = field.name || field.label || 'unknown';

    // 1. 필수 필드 검사
    if (value === undefined || value === null || value === '') {
        if (field.required) {
            return { valid: false, error: `required`, errorType: 'required', field: name };
        }
        return { valid: true };
    }

    // 2. 타입별 검증
    switch (field.field_type) {
        case 'number':
            return validateNumber(field, value);
        case 'boolean':
            return validateBoolean(field, value);
        case 'select':
            return validateSelect(field, value);
        case 'text':
        case 'password':
        case 'file':
            return { valid: true };
        default:
            return { valid: true };
    }
}

/**
 * 숫자 타입 검증: 파싱 가능 여부, min/max 범위
 */
function validateNumber(field, value) {
    const name = field.name || 'unknown';
    const num = Number(value);

    if (isNaN(num)) {
        return {
            valid: false,
            error: `type_mismatch_number`,
            errorType: 'type_mismatch',
            field: name,
        };
    }

    if (field.min != null && num < field.min) {
        return {
            valid: false,
            error: `out_of_range_min`,
            errorType: 'out_of_range',
            field: name,
            min: field.min,
            value: num,
        };
    }

    if (field.max != null && num > field.max) {
        return {
            valid: false,
            error: `out_of_range_max`,
            errorType: 'out_of_range',
            field: name,
            max: field.max,
            value: num,
        };
    }

    return { valid: true };
}

/**
 * 불리언 타입 검증
 */
function validateBoolean(field, value) {
    if (typeof value === 'boolean') return { valid: true };
    if (value === 'true' || value === 'false') return { valid: true };
    return {
        valid: false,
        error: `type_mismatch_boolean`,
        errorType: 'type_mismatch',
        field: field.name,
    };
}

/**
 * 프리셋(select) 타입 검증
 */
function validateSelect(field, value) {
    if (!field.options || field.options.length === 0) return { valid: true };
    const strVal = String(value);
    if (!field.options.includes(strVal)) {
        return {
            valid: false,
            error: `invalid_option`,
            errorType: 'invalid_option',
            field: field.name,
            options: field.options,
            value: strVal,
        };
    }
    return { valid: true };
}

/**
 * 모듈의 전체 설정 필드를 한꺼번에 검증합니다.
 *
 * @param {Array} fields - module.toml의 settings.fields 목록
 * @param {Object} values - 현재 설정값 맵 { fieldName: value }
 * @returns {Object} { valid: boolean, errors: { [fieldName]: { error, errorType, ... } } }
 */
export function validateAllSettings(fields, values) {
    const errors = {};
    let valid = true;

    for (const field of fields) {
        const result = validateSettingValue(field, values[field.name]);
        if (!result.valid) {
            errors[field.name] = result;
            valid = false;
        }
    }

    return { valid, errors };
}

/**
 * 모든 인스턴스 간의 포트 충돌을 검사합니다.
 * 특정 인스턴스의 포트가 다른 인스턴스와 겹치는지 확인합니다.
 *
 * 모듈별 프로토콜 정보(moduleProtocols)가 제공되면,
 * 해당 모듈이 실제 사용하는 프로토콜의 포트만 비교합니다.
 * 예: REST를 지원하지 않는 모듈의 rest_port는 충돌 검사에서 제외됩니다.
 *
 * @param {string} targetId - 검사 대상 인스턴스 ID
 * @param {Object} targetPorts - 대상 포트 { port, rcon_port, rest_port }
 * @param {Array} allServers - 전체 서버(인스턴스) 목록
 * @param {Object} [moduleProtocols] - 모듈별 지원 프로토콜 맵 { moduleName: ["rcon","rest",...] }
 * @param {string} [targetModule] - 대상 인스턴스의 모듈 이름
 * @returns {Array} 충돌 목록 [{ port, portType, conflictName, conflictId, conflictPortType }]
 */
export function checkPortConflicts(targetId, targetPorts, allServers, moduleProtocols, targetModule) {
    const conflicts = [];

    const targetSupported = moduleProtocols && targetModule ? moduleProtocols[targetModule] : null;
    const portEntries = buildActivePortEntries(targetPorts, targetSupported);

    if (portEntries.length === 0) return conflicts;

    for (const server of allServers) {
        if (server.id === targetId) continue;

        const otherSupported = moduleProtocols && server.module ? moduleProtocols[server.module] : null;
        const otherPorts = buildActivePortEntries(
            { port: server.port, rcon_port: server.rcon_port, rest_port: server.rest_port },
            otherSupported,
        );

        for (const tp of portEntries) {
            for (const op of otherPorts) {
                if (Number(tp.value) === Number(op.value)) {
                    conflicts.push({
                        port: Number(tp.value),
                        portType: tp.type,
                        conflictName: server.name,
                        conflictId: server.id,
                        conflictPortType: op.type,
                    });
                }
            }
        }
    }

    return conflicts;
}

/**
 * 모듈의 지원 프로토콜에 따라 실제 사용하는 포트만 반환합니다.
 * @param {Object} ports - { port, rcon_port, rest_port }
 * @param {Array|null} supported - 모듈 지원 프로토콜 배열 (null이면 모두 포함)
 * @returns {Array} [{ value, type }]
 */
function buildActivePortEntries(ports, supported) {
    const entries = [
        { value: ports.port, type: 'port' }, // game port는 항상 포함
    ];
    // rcon_port: 프로토콜 정보가 없거나 rcon을 지원할 때만 포함
    if (!supported || supported.includes('rcon')) {
        entries.push({ value: ports.rcon_port, type: 'rcon_port' });
    }
    // rest_port: 프로토콜 정보가 없거나 rest를 지원할 때만 포함
    if (!supported || supported.includes('rest')) {
        entries.push({ value: ports.rest_port, type: 'rest_port' });
    }
    return entries.filter((e) => e.value != null && e.value !== '' && !isNaN(Number(e.value)));
}

/**
 * 모듈 별명(module aliases) 충돌을 검사합니다.
 * 한 모듈의 별명이 다른 모듈의 별명과 겹치는지 확인합니다.
 *
 * @param {string} targetModule - 검사할 모듈 이름
 * @param {Array} targetAliases - 대상 모듈의 별명 목록
 * @param {Object} moduleAliasesPerModule - 모듈별 별명 정의 { moduleName: { module_aliases: [...] } }
 * @param {Object} discordModuleAliases - GUI에서 설정한 커스텀 별명 { moduleName: "alias1,alias2" }
 * @returns {Array} 충돌 목록 [{ alias, conflictModule }]
 */
export function checkAliasConflicts(targetModule, targetAliases, moduleAliasesPerModule, discordModuleAliases) {
    const conflicts = [];

    if (!targetAliases || targetAliases.length === 0) return conflicts;

    // 다른 모든 모듈의 별명을 수집
    const otherAliasMap = {}; // alias(lower) → moduleName

    for (const [moduleName, aliasData] of Object.entries(moduleAliasesPerModule || {})) {
        if (moduleName === targetModule) continue;

        // module.toml 기본 별명
        const baseAliases = aliasData?.module_aliases || [];
        for (const alias of baseAliases) {
            otherAliasMap[alias.toLowerCase()] = moduleName;
        }

        // 모듈 이름 자체도 별명으로 간주
        otherAliasMap[moduleName.toLowerCase()] = moduleName;
    }

    // GUI 커스텀 별명
    for (const [moduleName, customStr] of Object.entries(discordModuleAliases || {})) {
        if (moduleName === targetModule) continue;
        const customs = (customStr || '')
            .split(',')
            .map((a) => a.trim())
            .filter((a) => a.length > 0);
        for (const alias of customs) {
            otherAliasMap[alias.toLowerCase()] = moduleName;
        }
    }

    // 대상 별명과 비교
    for (const alias of targetAliases) {
        const lower = alias.toLowerCase();
        if (otherAliasMap[lower]) {
            conflicts.push({
                alias,
                conflictModule: otherAliasMap[lower],
            });
        }
    }

    return conflicts;
}
