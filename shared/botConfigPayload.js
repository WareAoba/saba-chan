/**
 * bot-config.json 저장 payload 조립 — 순수 함수, main.js와 테스트가 공유.
 *
 * Electron main 프로세스(main.js)의 `botConfig:save` IPC 핸들러가 렌더러
 * 요청과 기존 파일 내용을 병합해 디스크에 쓸 최종 객체를 만들 때 사용한다.
 *
 * 회귀 배경: 예전에는 이 조립 로직이 main.js 안에 화이트리스트 형태로
 * 인라인되어 있었고, instanceSettings 필드가 목록에서 누락되어 있었다.
 * 렌더러(useDiscordStore)는 정상적으로 instanceSettings를 payload에 담아
 * 보냈지만 main.js가 여기서 떨어뜨려 디스크에 절대 기록되지 않았고,
 * 결과적으로 데몬의 인스턴스별 노드/유저 권한 판정이 항상 빈 설정
 * (= 전부 허용) 기준으로 동작했다. 필드를 나열하는 화이트리스트 방식은
 * 같은 실수가 재발하기 쉬우므로, 여기 하나의 목록(PERSISTED_BOT_CONFIG_KEYS)만
 * 늘리면 저장 경로 전체에 반영되도록 모아둔다.
 */

/** 로컬 봇 시절의 레거시 필드 — 새 설정에는 절대 쓰지 않는다. */
const LEGACY_BOT_CONFIG_KEYS = [
    'token',
    'autoStart',
    'mode',
    'musicEnabled',
    'musicChannelId',
    'musicUISettings',
    'musicUiSettings',
];

/** bot-config.json에 영속되어야 하는 필드. 새 설정 추가 시 여기만 늘리면 된다. */
const PERSISTED_BOT_CONFIG_KEYS = [
    'moduleAliases',
    'commandAliases',
    'cloud',
    'nodeSettings',
    'instanceSettings',
    'cloudNodes',
    'cloudMembers',
];

const EMPTY_OBJECT_DEFAULTS = new Set(['moduleAliases', 'commandAliases', 'cloud', 'nodeSettings', 'instanceSettings', 'cloudMembers']);

/** 레거시 필드를 제거한 얕은 복사본을 반환한다. null/undefined는 빈 객체로 취급. */
function stripLegacyBotConfigFields(config) {
    const clean = { ...(config || {}) };
    for (const key of LEGACY_BOT_CONFIG_KEYS) {
        delete clean[key];
    }
    return clean;
}

/**
 * 렌더러 요청(config)과 기존 파일 내용(existing)을 병합해 저장할 payload를 만든다.
 * 우선순위: 요청값 > 기존 파일 값 > 기본값(객체는 {}, 배열은 [], prefix는 '!saba').
 * 필드 단위 얕은 병합이므로, 렌더러가 instanceSettings 전체를 보내면 그 값이
 * 기존 값을 완전히 대체한다 (항목 삭제/체크 해제가 그대로 반영됨).
 */
function buildBotConfigSavePayload(config, existing) {
    const existingClean = stripLegacyBotConfigFields(existing);
    const req = config || {};

    const payload = {
        ...existingClean,
        prefix: req.prefix !== undefined ? req.prefix : existingClean.prefix || '!saba',
    };

    for (const key of PERSISTED_BOT_CONFIG_KEYS) {
        const fallback = EMPTY_OBJECT_DEFAULTS.has(key) ? {} : [];
        payload[key] = req[key] !== undefined ? req[key] : existingClean[key] !== undefined ? existingClean[key] : fallback;
    }

    return payload;
}

module.exports = {
    LEGACY_BOT_CONFIG_KEYS,
    PERSISTED_BOT_CONFIG_KEYS,
    stripLegacyBotConfigFields,
    buildBotConfigSavePayload,
};
