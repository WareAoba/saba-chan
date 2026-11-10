import { useDiscordStore } from '../../../stores/useDiscordStore';
import { PrivacyErase, privacyEraseAvailable } from '../../discord/PrivacyErase';
import { ExtensionGuiSection, extensionGuiAvailable } from '../ExtensionGuiSection';
import { UnisolatedSection, unisolatedAvailable } from '../UnisolatedSection';

/** 설정 › 보안: 격리 없이 도는 것·익스텐션 화면 허락·디스코드 개인정보 지우기 */
export function SecurityTab() {
    const discordHostId = useDiscordStore((s) => s.discordCloudHostId);
    return (
        <>
            {unisolatedAvailable() && <UnisolatedSection />}

            {extensionGuiAvailable() && <ExtensionGuiSection />}

            {/* 디스코드 계정: 개인정보 지우기 (디스코드 화면에서 옮겨 옴). AI 에이전트 연결은 연결 탭에 있다 */}
            {discordHostId && privacyEraseAvailable() && (
                <section className="saba-card">
                    <PrivacyErase
                        className="setting-item"
                        onErased={() =>
                            // 데몬이 연결을 끊었다: 디스코드 화면이 연결 전으로 돌아가게 비운다
                            useDiscordStore
                                .getState()
                                .update({ discordCloudHostId: '', cloudNodes: [], cloudMembers: {} })
                        }
                    />
                </section>
            )}
        </>
    );
}
