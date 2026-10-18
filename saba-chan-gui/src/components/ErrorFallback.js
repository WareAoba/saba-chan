import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { WindowControls } from './shell/AppBar';
import { SabaArt } from './ui/SabaArt';

/**
 * Fallback UI rendered when an uncaught error is caught by ErrorBoundary.
 * Shows error details and a retry button.
 * App이 사라지면 종료 질문을 받을 곳도 없어진다: 창 버튼과 이동 영역을 두고, ✕는 화면만 닫는다(데몬은 유지).
 */
export function ErrorFallback({ error, resetErrorBoundary }) {
    const { t } = useTranslation('gui');

    useEffect(() => {
        if (!window.api?.onCloseRequest) return undefined;
        window.api.onCloseRequest(() => window.api.closeResponse?.('exit-interface'));
        return () => window.api.offCloseRequest?.();
    }, []);

    return (
        <div className="error-fallback" role="alert">
            <div className="appbar appbar-bare">
                <div className="appbar-spacer" />
                <WindowControls />
            </div>
            <div className="error-fallback-content">
                <SabaArt kind="stopped" size={180} />
                <h2>{t('errors.unexpected_crash')}</h2>
                <pre className="error-fallback-details">{error.message}</pre>
                <button type="button" className="btn btn-primary" onClick={resetErrorBoundary}>
                    {t('errors.retry')}
                </button>
            </div>
        </div>
    );
}
