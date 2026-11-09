import { type ReactNode, useLayoutEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../../types/daemon';
import { AboutPage } from '../AboutPage';

/** 데몬의 lastUpdated(unix 초, 문자열) → 날짜 글자. 읽을 수 없으면 null */
export function formatLastUpdated(secs: string | number | null | undefined, language: string) {
    const n = Number(secs);
    if (!secs || !Number.isFinite(n)) return null;
    return new Date(n * 1000).toLocaleDateString(language, { year: 'numeric', month: 'long', day: 'numeric' });
}

interface AboutTabProps {
    appVersion: string | null;
    componentInfo: ApiBody | null;
    onLogoClick: () => void;
}

/** 설정 › 정보: 사바쨩 소개와 버전 (한 화면에 담기게 줄인다) */
export function AboutTab({ appVersion, componentInfo, onLogoClick }: AboutTabProps) {
    const { i18n } = useTranslation(['gui', 'common']);
    return (
        <FitHeight>
            <AboutPage
                active
                appVersion={appVersion}
                componentInfo={componentInfo}
                lastUpdated={formatLastUpdated(componentInfo?.lastUpdated, i18n.language)}
                onLogoClick={onLogoClick}
            />
        </FitHeight>
    );
}

/**
 * 안의 내용이 스크롤 칸(.settings-modal-content)의 남은 높이를 넘으면 통째로 줄여(scale) 한 화면에 담는다.
 * 칸 크기가 바뀌거나 그림을 다 받으면 다시 잰다. 내용 자신의 크기 변화는 보지 않는다 (줄이면 다시 재는 고리가 생긴다)
 */
function FitHeight({ children }: { children: ReactNode }) {
    const ref = useRef<HTMLDivElement>(null);
    useLayoutEffect(() => {
        const el = ref.current;
        const box = el?.closest<HTMLElement>('.settings-modal-content');
        if (!el || !box) return;
        // zoom은 폭까지 다시 계산해 그림이 커지므로, 배치는 그대로 두고 그리기만 줄인다(transform)
        const fit = () => {
            const padBottom = Number.parseFloat(getComputedStyle(box).paddingBottom) || 0;
            const room = box.getBoundingClientRect().bottom - padBottom - el.getBoundingClientRect().top;
            const need = el.offsetHeight;
            const scale = need > room && room > 0 ? Math.max(0.4, room / need) : 1;
            el.style.transform = scale < 1 ? `scale(${scale})` : '';
        };
        fit();
        // 테스트 환경(jsdom)에는 ResizeObserver가 없다
        const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(fit);
        observer?.observe(box);
        el.addEventListener('load', fit, true);
        return () => {
            observer?.disconnect();
            el.removeEventListener('load', fit, true);
        };
    }, []);
    return (
        <div ref={ref} className="fit-height" style={{ transformOrigin: 'top center' }}>
            {children}
        </div>
    );
}
