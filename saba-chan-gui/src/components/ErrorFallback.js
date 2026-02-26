
import { useTranslation } from 'react-i18next';

/**
 * Fallback UI rendered when an uncaught error is caught by ErrorBoundary.
 * Shows error details and a retry button.
 */
export function ErrorFallback({ error, resetErrorBoundary }) {
    const { t } = useTranslation('gui');

    return (
        <div className="error-fallback" role="alert">
            <div className="error-fallback-content">
                <h2>{t('errors.unexpected_crash', 'Something went wrong')}</h2>
                <pre className="error-fallback-details">{error.message}</pre>
                <button className="btn btn-primary" onClick={resetErrorBoundary}>
                    {t('errors.retry', 'Try Again')}
                </button>
            </div>
        </div>
    );
}
