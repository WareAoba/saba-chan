import { useTranslation } from 'react-i18next';
import { art } from '../art';
import { useApp } from '../store';
import { Icon } from './icons';

/** 내려받는 곳 — 사이트 첫 화면. 웹은 사이트와 같은 호스트의 /remote/ 아래에 놓이므로 이 화면을 연 곳이다 */
const SiteLink = () => (
    <a href={window.location.origin} target="_blank" rel="noopener noreferrer">
        {window.location.host}
    </a>
);

/** PC 없는 세션 위쪽의 설치 안내 띠 (접속할 때마다 한 번, 닫을 수 있다) */
export function InstallBand() {
    const { t } = useTranslation();
    const closed = useApp((s) => s.installBandClosed);
    const close = useApp((s) => s.closeInstallBand);
    if (closed) return null;
    return (
        <aside className="install-band">
            <Icon name="download" size={18} />
            <p>
                {t('no_pc.band')} <SiteLink />
            </p>
            <button type="button" className="icon-btn" aria-label={t('no_pc.band_close')} onClick={close}>
                <Icon name="close" size={16} />
            </button>
        </aside>
    );
}

/** 잠긴 탭(서버·모듈·PC)을 누르면 보이는 화면 */
export function NeedsPc() {
    const { t } = useTranslation();
    return (
        <section className="needs-pc">
            <img className="empty-art" src={art.relay} alt="" />
            <h2>{t('no_pc.title')}</h2>
            <p className="hint">{t('no_pc.text')}</p>
            <ol className="needs-pc-steps">
                <li>
                    {t('no_pc.step_download')} <SiteLink />
                </li>
                <li>{t('no_pc.step_connect')}</li>
                <li>{t('no_pc.step_link')}</li>
            </ol>
        </section>
    );
}
