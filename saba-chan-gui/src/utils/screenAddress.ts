/**
 * 화면 주소 (사용자 11-06 #19): 안내 문구의 링크가 가리키는 화면. 앱과 웹이 같은 주소를 쓴다.
 *
 *   server/<서버 id>[/<탭>]                    서버 상세의 탭 (console·commands·settings·saba·maintenance·discord·info)
 *   discord[/connection | /saba]                디스코드 › 연결 · 사바쨩 설정
 *   discord/<디스코드 서버 id | *>[/<탭>[/<안 화면>]]
 *                                               디스코드 서버의 탭 (instances·members·moderation·bot·chatbot·music·tts)과
 *                                               그 안 화면 (moderation/channels·members). *는 연결된 디스코드 서버가 하나일 때 그 서버
 *   settings[/<탭>]                             설정의 탭
 *   activity                                    작업 기록
 *
 * 읽기만 한다. 화면을 여는 일은 앱(utils/openScreen)과 웹이 각자 한다.
 */
export type ScreenAddress =
    | { page: 'server'; id: string; tab?: string }
    | { page: 'discord'; view: 'connection' | 'saba' }
    | { page: 'discord'; view: 'node'; id: string; tab?: string; sub?: string }
    | { page: 'settings'; tab?: string }
    | { page: 'activity' };

// id·탭 이름에 쓰는 글자만 받는다 (빈 칸·공백·주소 조각이 섞인 것은 주소가 아니다)
const SEGMENT = /^[A-Za-z0-9_.*-]+$/;

/** 주소 → 화면. 모르는 화면이거나 칸이 비었으면 null */
export function parseScreen(address: string): ScreenAddress | null {
    const parts = String(address || '').split('/');
    if (!parts.every((p) => SEGMENT.test(p))) return null;
    const [page, a, b, c] = parts;
    const opt = <K extends string>(key: K, v?: string) => (v ? ({ [key]: v } as Record<K, string>) : {});
    switch (page) {
        case 'server':
            if (!a || parts.length > 3) return null;
            return { page: 'server', id: a, ...opt('tab', b) };
        case 'discord':
            if (parts.length === 1 || (parts.length === 2 && a === 'connection'))
                return { page: 'discord', view: 'connection' };
            if (parts.length === 2 && a === 'saba') return { page: 'discord', view: 'saba' };
            if (parts.length > 4) return null;
            return { page: 'discord', view: 'node', id: a, ...opt('tab', b), ...opt('sub', c) };
        case 'settings':
            if (parts.length > 2) return null;
            return { page: 'settings', ...opt('tab', a) };
        case 'activity':
            return parts.length === 1 ? { page: 'activity' } : null;
        default:
            return null;
    }
}
