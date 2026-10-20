/**
 * 설정값 타입 검증 및 포트/별명 충돌 검사 유틸리티
 *
 * - validateSettingValue: module.toml SettingField 스키마 기반 개별 필드 검증
 * - validateAllSettings: 전체 설정 필드 일괄 검증
 * - checkPortConflicts: 모든 인스턴스 간 포트 충돌 검사
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

    if (field.field_type === 'list') return validateList(field, listItems(value));

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
        case 'multiline':
            return field.max_length != null && String(value).length > field.max_length
                ? { valid: false, errorType: 'too_long', field: name, max: field.max_length }
                : { valid: true };
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

/** list 칸 값: 문자열 배열에서 빈 항목을 뺀다 (편집 중에는 빈 줄이 남는다). 저장할 때도 이 값을 보낸다 */
export function listItems(value) {
    const items = Array.isArray(value) ? value : value == null || value === '' ? [] : String(value).split(',');
    return items.map((v) => String(v).trim()).filter(Boolean);
}

/** 목록 검증: 항목 수(min_items·max_items), 선택지가 있으면 항목마다 그중 하나 */
function validateList(field, items) {
    const name = field.name || 'unknown';
    if (items.length === 0 && field.required) return { valid: false, errorType: 'required', field: name };
    const tooFew = field.min_items != null && items.length < field.min_items;
    const tooMany = field.max_items != null && items.length > field.max_items;
    if (tooFew || tooMany) {
        return { valid: false, errorType: 'item_count', field: name, min: field.min_items, max: field.max_items };
    }
    const bad = field.options?.length ? items.find((v) => !field.options.includes(v)) : undefined;
    if (bad !== undefined) return { valid: false, errorType: 'invalid_option', field: name, value: bad };
    return { valid: true };
}

/** 설정 타입으로 바꾼 값끼리 같은가 (데몬 validator::same_setting_value) */
function sameSettingValue(fieldType, a, b) {
    const flag = (v) => {
        if (typeof v === 'boolean') return v;
        const word = String(v).trim();
        return word === 'true' ? true : word === 'false' ? false : null;
    };
    const num = (v) => (typeof v === 'number' ? v : typeof v === 'string' && v.trim() !== '' ? Number(v.trim()) : NaN);
    if (fieldType === 'boolean') return flag(a) !== null && flag(a) === flag(b);
    if (fieldType === 'number') return !Number.isNaN(num(a)) && num(a) === num(b);
    const text = (v) => (typeof v === 'string' ? v.trim() : JSON.stringify(v));
    return text(a) === text(b);
}

/**
 * `visible_when`으로 지금 숨겨진 설정 이름 (데몬 validator::hidden_fields와 같은 계산, 02 §1.2).
 * 값이 없으면(빈 칸 포함) 그 설정의 default로 비교하고, default도 없으면 숨긴다.
 */
export function hiddenFields(fields, values) {
    return (fields || [])
        .filter((f) => {
            const cond = f.visible_when;
            if (!cond) return false;
            const target = fields.find((t) => t.name === cond.field);
            if (!target) return false;
            const raw = values?.[target.name];
            const current = raw === undefined || raw === null || raw === '' ? target.default : raw;
            if (current === undefined || current === null) return true;
            const wanted = Array.isArray(cond.in) ? cond.in : [cond.equals];
            return !wanted.some((want) => sameSettingValue(target.field_type, current, want));
        })
        .map((f) => f.name);
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
    // 숨긴 설정은 required를 보지 않는다 (데몬과 같다)
    const hidden = new Set(hiddenFields(fields, values));

    for (const field of fields) {
        if (hidden.has(field.name)) continue;
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
 * @param {Object} targetPorts - 대상 포트 { port, rest_port }
 * @param {Array} allServers - 전체 서버(인스턴스) 목록
 * @param {Object} [moduleProtocols] - 모듈별 지원 프로토콜 맵 { moduleName: ["rest","stdin",...] }
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
        const otherPorts = buildActivePortEntries({ port: server.port, rest_port: server.rest_port }, otherSupported);

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
 * @param {Object} ports - { port, rest_port }
 * @param {Array|null} supported - 모듈 지원 프로토콜 배열 (null이면 모두 포함)
 * @returns {Array} [{ value, type }]
 */
function buildActivePortEntries(ports, supported) {
    const entries = [
        { value: ports.port, type: 'port' }, // game port는 항상 포함
    ];
    // rest_port: 프로토콜 정보가 없거나 rest를 지원할 때만 포함
    if (!supported || supported.includes('rest')) {
        entries.push({ value: ports.rest_port, type: 'rest_port' });
    }
    return entries.filter((e) => e.value != null && e.value !== '' && !isNaN(Number(e.value)));
}
