import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/** 웹이 GUI 에서 가져다 쓰는 것. 아이콘·드롭다운 말고 화면(컴포넌트)은 가져오지 않는다 — 웹은 할 수 있는 일이 달라 따로 만든다 */
const ALLOWED = new Set([
    '@gui/api/createApi',
    '@gui/api/events',
    '@gui/utils/serverView',
    '@gui/utils/localizedText',
    '@gui/utils/commandRisk',
    // 명령 줄 읽기와 입력 칸 값 → 인자 (데몬이 읽는 방식 그대로, 앱 명령 탭과 같은 규칙)
    '@gui/utils/commandSyntax',
    // 포트포워딩 확인 결과 읽기 (화면 없는 순수 함수)
    '@gui/utils/portCheck',
    // 예약 작업 초안 ↔ 데몬 형식과 검사 (GUI 백업·예약 탭과 같은 규칙)
    '@gui/utils/schedule',
    // 활동 기록 한 건을 문구로 (앱 작업 기록과 같은 규칙)
    '@gui/utils/audit',
    // 화면 주소 읽기 (앱과 같은 주소, 화면 없는 순수 함수)
    '@gui/utils/screenAddress',
    // 고를 수 있는 언어 목록 (이 화면 설정)
    '@gui/utils/languages',
    // 콘솔 글꼴 후보·기본값과 글꼴 묶음 (이 화면 설정, 앱 설정과 같은 목록)
    '@gui/utils/consoleFonts',
    // 새 서버 만들기 규칙 (설치할 수 있는 값, 기본 이름, 이름 겹침)
    '@gui/utils/installFields',
    // 권한 단계 계산과 사다리 편집 (화면 없는 순수 함수)
    '@gui/components/discord/levels',
    // 칩이 자리를 바꾸는 움직임 (권한 맞바꾸기, 앱 사다리와 같은 움직임)
    '@gui/components/discord/flip',
    // 마우스·손가락으로 칩 끌어 놓기 (화면 없는 훅, 앱 계단과 같은 끌기)
    '@gui/components/discord/pointerDrag',
    // 인스턴스 낱말 초안과 검사 오류 나누기 (화면 없는 순수 함수)
    '@gui/components/discord/instanceWords',
    // 조작 가능 명단 넣기·빼기·멤버 고르기 (GUI 조작 가능 탭과 같은 순수 함수)
    '@gui/components/servers/controllers',
    // 디스코드 서버별 보일 서버 규칙 (GUI 노드 인스턴스 탭과 같은 순수 함수)
    '@gui/components/discord/nodePermissions',
    // 코코낸내 기능 묶음·시각 (GUI 봇 설정과 같은 상수)
    '@gui/components/discord/sleep',
    // 데몬·디스코드 응답 모양 (타입만, 실행 코드 없음)
    '@gui/types/daemon',
    '@gui/components/discord/types',
    '@gui/styles/tokens.css',
    // 그라데이션 버튼 호버 순환 (앱과 같은 움직임과 그림)
    '@gui/utils/gradCycle',
    '@gui/styles/grad-cycle.css',
    // 아이콘은 GUI 와 같은 것을 쓴다 (화면이 아니라 그림 하나)
    '@gui/components/Icon',
    '@gui/components/CustomDropdown/CustomDropdown',
    // 체크박스는 GUI 와 같은 것을 쓴다 (화면이 아니라 입력 부품 하나)
    '@gui/components/ui/SabaUI',
    // 명령 입력 줄 (친 글자 색칠·안내 부품, 스토어·API 없이 받은 상태만 그린다)
    '@gui/components/ui/CommandLine',
    // PC 상태 원과 항목 고르기: 앱 시스템 모니터를 웹에서 그대로 본다 (스토어 없는 그림 부품과 순수 계산)
    '@gui/components/monitor/MonitorRing',
    '@gui/components/monitor/systemMetrics',
]);

function sources(dir: string): string[] {
    return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
        const p = join(dir, e.name);
        if (e.isDirectory()) return e.name === 'test' ? [] : sources(p);
        return /\.(jsx?|tsx?|css)$/.test(e.name) ? [p] : [];
    });
}

describe('GUI 에서 가져오는 것', () => {
    it('정해 둔 목록만 가져온다', () => {
        const found = new Set<string>();
        for (const file of sources(join(__dirname, '..'))) {
            for (const m of readFileSync(file, 'utf8').matchAll(/['"](@gui\/[^'"]+)['"]/g)) found.add(m[1]);
        }
        expect([...found].filter((p) => !ALLOWED.has(p))).toEqual([]);
    });
});
