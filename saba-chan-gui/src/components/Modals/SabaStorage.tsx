import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ComponentsTab } from './SabaStorageComponents';
import { ExtensionsTab } from './SabaStorageExtensions';
import { ModulesTab } from './SabaStorageModules';

// ─────────────────────────────────────────────────────────────
// 사바 스토리지 (구 업데이트 센터)
//
// 탭 3개:
//   [컴포넌트] — 앱 구성요소 업데이트 (SabaStorageComponents.tsx)
//   [모듈]     — 설치된 모듈 + saba-chan-modules 레지스트리 (SabaStorageModules.tsx)
//   [익스텐션] — 기존 익스텐션 탭 내용 이전 (SabaStorageExtensions.tsx)
//
// @param {boolean}  devMode   - 개발자 모드
// @param {string}   initialTab - 처음 보일 탭 (components | modules | extensions)
// ─────────────────────────────────────────────────────────────
interface SabaStorageProps {
    devMode?: boolean;
    initialTab?: string;
}

function SabaStorage({ devMode, initialTab = 'components' }: SabaStorageProps) {
    const { t } = useTranslation('gui');
    const [activeTab, setActiveTab] = useState(initialTab);
    const tabsRef = useRef<HTMLDivElement>(null);
    const indicatorRef = useRef<HTMLDivElement>(null);
    const syncIndicator = useCallback(() => {
        const container = tabsRef.current;
        const indicator = indicatorRef.current;
        if (!container || !indicator) return;
        const activeBtn = container.querySelector<HTMLElement>('.saba-storage-tab.active');
        if (!activeBtn) return;
        indicator.style.left = `${activeBtn.offsetLeft}px`;
        indicator.style.width = `${activeBtn.offsetWidth}px`;
    }, []);
    // biome-ignore lint/correctness/useExhaustiveDependencies: activeTab triggers DOM re-render that syncIndicator reads via querySelector
    useEffect(() => {
        syncIndicator();
    }, [activeTab, syncIndicator]);

    return (
        // 설정의 한 탭으로 열린다: 제목(h3)은 설정 쪽이 다른 탭과 같은 모양으로 그린다
        <div className="saba-storage">
            {/* 내부 탭 */}
            <div className="saba-storage-tabs" ref={tabsRef}>
                <div className="saba-storage-tab-indicator" ref={indicatorRef} />
                {['components', 'modules', 'extensions'].map((tab) => (
                    <button
                        key={tab}
                        data-spot={`storage.tab.${tab}`}
                        className={clsx('saba-storage-tab', { active: activeTab === tab })}
                        onClick={() => setActiveTab(tab)}
                    >
                        {t(
                            `saba_storage.tab_${tab}`,
                            tab === 'components' ? '컴포넌트' : tab === 'modules' ? '모듈' : '익스텐션',
                        )}
                    </button>
                ))}
            </div>

            {/* 세 탭을 처음부터 모두 띄워 두고 숨기기만 한다: 탭을 바꿀 때마다 목록을 다시 받지 않는다 */}
            <div className="saba-storage-content">
                <div className="saba-storage-pane" hidden={activeTab !== 'components'}>
                    <ComponentsTab devMode={devMode} />
                </div>
                <div className="saba-storage-pane" hidden={activeTab !== 'modules'}>
                    <ModulesTab />
                </div>
                <div className="saba-storage-pane" hidden={activeTab !== 'extensions'}>
                    <ExtensionsTab />
                </div>
            </div>
        </div>
    );
}

export default SabaStorage;
