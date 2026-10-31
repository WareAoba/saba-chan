import { gameNameOf, initials } from '@gui/utils/serverView';
import { useTranslation } from 'react-i18next';

/** 모듈 아이콘 (데몬이 준 data: 그림). 없으면 게임 이름 첫 글자 */
export function ServerIcon({ module, moduleData, size = 40 }) {
    const { t } = useTranslation();
    const icon =
        typeof moduleData?.icon === 'string' && moduleData.icon.startsWith('data:image/') ? moduleData.icon : null;
    return (
        <span className="server-icon" style={{ width: size, height: size }} aria-hidden="true">
            {icon ? <img src={icon} alt="" /> : initials(gameNameOf(t, module, moduleData))}
        </span>
    );
}
