/**
 * bot-config 저장 payload 조립 테스트 (shared/botConfigPayload.js)
 *
 * Electron main의 botConfig:save IPC 핸들러가 사용하는 순수 함수를 검증한다.
 * 회귀 배경: 핸들러의 화이트리스트에 instanceSettings가 빠져 있어
 * 서버별(인스턴스별) 노드/유저 권한이 디스크에 저장되지 않았다 —
 * 렌더러/데몬 단위 테스트만으로는 main 프로세스의 필드 누락을 잡지 못한다.
 */
import { describe, expect, it } from 'vitest';
import {
    LEGACY_BOT_CONFIG_KEYS,
    buildBotConfigSavePayload,
    stripLegacyBotConfigFields,
} from '../../../shared/botConfigPayload';

describe('buildBotConfigSavePayload — 영속 필드 보존', () => {
    it('instanceSettings가 요청값 그대로 payload에 포함된다 (권한 미저장 회귀)', () => {
        const instanceSettings = {
            'srv-1': {
                aliases: ['팰월드'],
                commandAliases: { start: ['기동'] },
                allowedNodes: ['guild-a'],
                blockedUsers: { stop: ['user-1'] },
            },
        };
        const payload = buildBotConfigSavePayload({ instanceSettings }, {});
        expect(payload.instanceSettings).toEqual(instanceSettings);
    });

    it('요청에 instanceSettings가 없으면 기존 파일 값을 보존한다', () => {
        const existing = { instanceSettings: { 'srv-1': { allowedNodes: [] } } };
        const payload = buildBotConfigSavePayload({ prefix: '!x' }, existing);
        expect(payload.instanceSettings).toEqual(existing.instanceSettings);
    });

    it('요청·기존 모두 없으면 빈 객체로 채운다', () => {
        const payload = buildBotConfigSavePayload({}, {});
        expect(payload.instanceSettings).toEqual({});
    });

    it('렌더러가 보내는 모든 영속 필드가 payload에 존재한다', () => {
        const payload = buildBotConfigSavePayload({}, {});
        for (const key of [
            'prefix',
            'cloud',
            'moduleAliases',
            'commandAliases',
            'nodeSettings',
            'instanceSettings',
            'cloudNodes',
            'cloudMembers',
        ]) {
            expect(payload, `payload에 ${key} 누락`).toHaveProperty(key);
        }
    });

    it('요청값이 기존값을 대체한다 (얕은 병합 — 인스턴스 항목 삭제 반영)', () => {
        const existing = {
            instanceSettings: { 'srv-1': { allowedNodes: ['g1'] }, 'srv-2': { allowedNodes: ['g2'] } },
        };
        // srv-2 설정을 전부 해제(항목 제거)한 상태로 저장
        const payload = buildBotConfigSavePayload(
            { instanceSettings: { 'srv-1': { allowedNodes: ['g1'] } } },
            existing,
        );
        expect(payload.instanceSettings['srv-2']).toBeUndefined();
    });

    it('기존 파일의 알 수 없는 필드는 그대로 보존한다', () => {
        const payload = buildBotConfigSavePayload({}, { futureField: { a: 1 } });
        expect(payload.futureField).toEqual({ a: 1 });
    });

    it('prefix는 요청 → 기존 → 기본값(!saba) 순으로 선택된다', () => {
        expect(buildBotConfigSavePayload({ prefix: '!p' }, { prefix: '!e' }).prefix).toBe('!p');
        expect(buildBotConfigSavePayload({}, { prefix: '!e' }).prefix).toBe('!e');
        expect(buildBotConfigSavePayload({}, {}).prefix).toBe('!saba');
    });
});

describe('stripLegacyBotConfigFields — 레거시 필드 제거', () => {
    it('로컬 봇 시절 레거시 필드를 모두 제거한다', () => {
        const legacy = Object.fromEntries(LEGACY_BOT_CONFIG_KEYS.map((k) => [k, 'x']));
        const clean = stripLegacyBotConfigFields({ ...legacy, prefix: '!keep' });
        for (const key of LEGACY_BOT_CONFIG_KEYS) {
            expect(clean).not.toHaveProperty(key);
        }
        expect(clean.prefix).toBe('!keep');
    });

    it('null/undefined 입력은 빈 객체로 처리한다', () => {
        expect(stripLegacyBotConfigFields(null)).toEqual({});
        expect(stripLegacyBotConfigFields(undefined)).toEqual({});
    });
});
