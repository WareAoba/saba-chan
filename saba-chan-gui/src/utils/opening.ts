/**
 * 오프닝(로딩 화면이 걷히는 순간)과 정상 기동음.
 *
 * 기동음은 데몬과 릴레이가 둘 다 정상으로 붙었을 때 오프닝과 함께 한 번 낸다. 릴레이 첫 확인은 데몬 준비 뒤에
 * 오므로, 그 답을 받을 때까지(최대 RELAY_WAIT_MS) 오프닝을 붙잡아 소리와 애니메이션이 같이 시작하게 한다.
 * 릴레이를 아직 연결하지 않았으면 실패한 것이 없으니 데몬만으로 낸다.
 */
export const RELAY_WAIT_MS = 4000;

export interface OpeningInput {
    /** 데몬에 붙어서 연 것인가 (데몬 없이 '그대로 열기'는 false) */
    daemonOk: boolean;
    /** 봇 설정(릴레이 호스트)을 읽었는가 */
    configLoaded: boolean;
    relayConfigured: boolean;
    /** 릴레이 첫 확인이 답했는가 */
    relayChecked: boolean;
    relayConnected: boolean;
    /** 데몬 준비 뒤 릴레이를 기다린 시간 */
    waitedMs: number;
}

export function openingGate(s: OpeningInput): { hold: boolean; chime: boolean } {
    if (!s.daemonOk) return { hold: false, chime: false };
    const relayUnknown = !s.configLoaded || (s.relayConfigured && !s.relayChecked);
    if (relayUnknown && s.waitedMs < RELAY_WAIT_MS) return { hold: true, chime: false };
    if (relayUnknown) return { hold: false, chime: false };
    return { hold: false, chime: !s.relayConfigured || s.relayConnected };
}
