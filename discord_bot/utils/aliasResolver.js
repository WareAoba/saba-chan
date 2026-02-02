/**
 * Discord Bot 별명 해석 유틸리티
 * 모듈 별명과 명령어 별명을 통합 관리
 */

/**
 * 모듈 별명 맵 생성 (GUI + TOML + default)
 * @param {object} botConfig - bot-config.json
 * @param {object} moduleMetadata - 모듈 메타데이터
 * @returns {object} { alias: moduleName } 형태의 맵
 */
function buildModuleAliasMap(botConfig, moduleMetadata) {
    const combined = {};
    
    // 1. 기본값: 모듈 이름 자체
    for (const moduleName of Object.keys(moduleMetadata)) {
        combined[moduleName] = moduleName;
    }
    
    // 2. module.toml의 [aliases].module_aliases
    for (const [moduleName, metadata] of Object.entries(moduleMetadata)) {
        if (metadata.aliases && metadata.aliases.module_aliases) {
            for (const alias of metadata.aliases.module_aliases) {
                combined[alias] = moduleName;
            }
        }
    }
    
    // 3. GUI에서 설정한 커스텀 별명
    for (const [moduleName, customAlias] of Object.entries(botConfig.moduleAliases || {})) {
        const aliasStr = (customAlias || '').trim();
        if (aliasStr.length > 0) {
            // 콤마로 구분된 여러 별명 지원
            const aliases = aliasStr.split(',').map(a => a.trim()).filter(a => a.length > 0);
            for (const alias of aliases) {
                combined[alias] = moduleName;
            }
        }
    }
    
    return combined;
}

/**
 * 명령어 별명 맵 생성 (GUI + TOML + default)
 * @param {object} botConfig - bot-config.json
 * @param {object} moduleMetadata - 모듈 메타데이터
 * @returns {object} { alias: commandName } 형태의 맵
 */
function buildCommandAliasMap(botConfig, moduleMetadata) {
    const combined = {};
    
    // 1. 기본 명령어들
    const defaultCommands = ['start', 'stop', 'status'];
    for (const cmd of defaultCommands) {
        combined[cmd] = cmd;
    }
    
    // 2. module.toml의 [aliases].commands
    for (const [moduleName, metadata] of Object.entries(moduleMetadata)) {
        if (metadata.aliases && metadata.aliases.commands) {
            for (const [cmdName, cmdData] of Object.entries(metadata.aliases.commands)) {
                // 명령어 이름 자체를 별명으로 추가
                combined[cmdName] = cmdName;
                
                // 별명 배열 추출 (legacy 배열 형식과 새 객체 형식 모두 지원)
                const aliases = cmdData.aliases || (Array.isArray(cmdData) ? cmdData : []);
                for (const alias of aliases) {
                    combined[alias] = cmdName;
                }
            }
        }
    }
    
    // 3. GUI에서 설정한 커스텀 별명
    for (const [moduleName, moduleCommands] of Object.entries(botConfig.commandAliases || {})) {
        if (typeof moduleCommands === 'object' && moduleCommands !== null) {
            for (const [cmdName, aliasStr] of Object.entries(moduleCommands)) {
                // 명령어 이름 자체
                combined[cmdName] = cmdName;
                
                // 콤마로 구분된 별명들
                if (typeof aliasStr === 'string' && aliasStr.trim().length > 0) {
                    const aliases = aliasStr.split(',').map(a => a.trim()).filter(a => a.length > 0);
                    for (const alias of aliases) {
                        combined[alias] = cmdName;
                    }
                }
            }
        }
    }
    
    return combined;
}

/**
 * 별명을 실제 이름으로 변환 (대소문자 무시)
 * @param {string} input - 입력된 별명
 * @param {object} aliasMap - 별명 맵
 * @returns {string} 실제 이름 (찾지 못하면 입력값 그대로)
 */
function resolveAlias(input, aliasMap) {
    const lowerInput = input.toLowerCase();
    
    // 별명으로 검색 (대소문자 무시)
    for (const [alias, actualName] of Object.entries(aliasMap)) {
        if (alias.toLowerCase() === lowerInput) {
            return typeof actualName === 'string' ? actualName : String(actualName);
        }
    }
    
    // 이미 실제 이름인지 확인
    const values = Object.values(aliasMap);
    for (const value of values) {
        if (typeof value === 'string' && value.toLowerCase() === lowerInput) {
            return value;
        }
    }
    
    // 찾지 못하면 입력값 그대로 반환
    return input;
}

module.exports = {
    buildModuleAliasMap,
    buildCommandAliasMap,
    resolveAlias
};
