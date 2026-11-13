import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { SabaSpinner } from '../ui/SabaUI';
import { SabaFail } from '../ui/SabaArt';
import * as L from './levels';

/* 권한 화면들(멤버·명령 계단)이 함께 쓰는 조각 */

export function permissionsErrorText(res: ApiBody | null | undefined, t: TFunction) {
    switch (res?.error_code || res?.code) {
        case 'owner_fixed':
        case 'OWNER_FIXED':
            return t('discord.perm_err_owner_fixed');
        case 'invalid_level':
            return t('discord.perm_err_invalid_level');
        case 'invalid_ladder':
            return t('discord.perm_err_invalid_ladder');
        default:
            return res?.error || t('discord.perm_err_generic');
    }
}

interface LevelSelectProps {
    value: number;
    levels: number[];
    perm: L.GuildPermissions;
    onChange: (level: number) => void;
    label: string;
    disabled?: boolean;
}

export function LevelSelect({ value, levels, perm, onChange, label, disabled }: LevelSelectProps) {
    const { t } = useTranslation('gui');
    return (
        <CustomDropdown
            className="dc-level-select"
            value={String(value)}
            ariaLabel={label}
            disabled={disabled}
            onChange={(v) => onChange(Number(v))}
            options={levels.map((l) => ({ value: String(l), label: L.levelTitle(t, perm, l) }))}
        />
    );
}

interface PermStateProps {
    loading: boolean;
    error: ApiBody | null;
    onRetry: () => void;
}

export function PermState({ loading, error, onRetry }: PermStateProps) {
    const { t } = useTranslation('gui');
    if (loading) {
        return (
            <div className="dc-pair-wait pad">
                <SabaSpinner size="sm" /> {t('discord.perm_loading')}
            </div>
        );
    }
    return <SabaFail message={permissionsErrorText(error, t)} onRetry={onRetry} retryLabel={t('discord.reload')} />;
}
