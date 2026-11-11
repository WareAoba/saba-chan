import { useSyncExternalStore } from 'react';
import { useTranslation } from 'react-i18next';
import { useSettingsStore } from '../../stores/useSettingsStore';
import {
    type WindowBackdropConfig,
    applyWindowBackdrop,
    getBackdropSupport,
    subscribeBackdropSupport,
} from '../../utils/themeManager';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaSlider, SabaToggle } from '../ui/SabaUI';

const WINDOWS_CHOICES = ['tabbed', 'mica', 'acrylic'];
const MAC_CHOICES = ['regular', 'clear'];

/**
 * 설정 > 외관: 창 바탕 반투명 (사용자 11-08). 켜고 끄기, 재질(Windows·macOS), 덮는 바탕의 진하기·밝기.
 * 셸이 OS 효과를 깔고(backdrop.rs) 이 화면이 그 위를 테마 바탕으로 덮는다. 바꾸면 바로 저장·적용한다
 */
export function WindowBackdropSetting() {
    const { t } = useTranslation('gui');
    const backdrop = useSettingsStore((s) => s.windowBackdrop);
    const support = useSyncExternalStore(subscribeBackdropSupport, getBackdropSupport);

    const change = (partial: Partial<WindowBackdropConfig>) => {
        const next = { ...backdrop, ...partial };
        useSettingsStore.getState().update({ windowBackdrop: next });
        void applyWindowBackdrop(next);
    };

    const platform = support?.platform;
    const unavailable = support ? !support.available : false;
    const choices = platform === 'macos' ? MAC_CHOICES : platform === 'linux' ? [] : (support?.choices ?? WINDOWS_CHOICES);
    const materialKey = platform === 'macos' ? 'mac' : 'windows';
    const material = platform === 'macos' ? backdrop.mac : backdrop.windows;
    const minOpacity = support?.minOpacity ?? 0;

    const description = () => {
        if (unavailable) return t(`settings_modal.backdrop_unavailable_${(support?.reason || 'other').replace(/-/g, '_')}`);
        if (platform === 'linux') return t('settings_modal.backdrop_desc_linux');
        if (platform === 'macos') return t('settings_modal.backdrop_desc_macos');
        return t(material === 'acrylic' ? 'settings_modal.backdrop_desc_acrylic' : 'settings_modal.backdrop_desc_mica');
    };
    // 켜고 끄기가 창을 다시 열 때 적용되는 곳(Linux): 설정과 지금 창이 다르면 알린다
    const needsReopen = !!support && !support.liveToggle && backdrop.enabled !== support.active && support.available;

    return (
        <div className="setting-item setting-item-group">
            <div className="setting-item-row">
                <div className="setting-label">
                    <span className="setting-title">
                        {t('settings_modal.backdrop_label')}
                    </span>
                    <span className="setting-description">{description()}</span>
                </div>
                <SabaToggle
                    checked={backdrop.enabled && !unavailable}
                    disabled={unavailable}
                    onChange={(on) => change({ enabled: on })}
                    aria-label={t('settings_modal.backdrop_label')}
                />
            </div>
            {needsReopen && (
                <div className="setting-restart-notice">
                    <Icon name="info" size="sm" /> {t('settings_modal.backdrop_reopen')}
                </div>
            )}
            {backdrop.enabled && !unavailable && (
                <div className="setting-sublist">
                    {choices.length > 0 && (
                        <div className="setting-subitem">
                            <span>{t('settings_modal.backdrop_material_label')}</span>
                            <CustomDropdown
                                className="setting-select"
                                value={choices.includes(material) ? material : choices[0]}
                                onChange={(val) => change({ [materialKey]: String(val) })}
                                options={choices.map((c) => ({
                                    value: c,
                                    label: t(`settings_modal.backdrop_material_${materialKey === 'mac' ? `mac_${c}` : c}`),
                                }))}
                            />
                        </div>
                    )}
                    {support?.slow.includes(material) && (
                        <div className="setting-description">{t('settings_modal.backdrop_slow')}</div>
                    )}
                    <div className="setting-subitem">
                        <span>{t('settings_modal.backdrop_opacity_label')}</span>
                        <div className="font-scale-row backdrop-slider">
                            <SabaSlider
                                min={0}
                                max={100}
                                step={5}
                                value={backdrop.opacity}
                                aria-label={t('settings_modal.backdrop_opacity_label')}
                                onChange={(v) => change({ opacity: v })}
                            />
                            <span className="font-scale-value">{Math.max(backdrop.opacity, minOpacity)}%</span>
                        </div>
                    </div>
                    {minOpacity > 0 && backdrop.opacity < minOpacity && (
                        <div className="setting-description">{t('settings_modal.backdrop_min_opacity', { min: minOpacity })}</div>
                    )}
                    <div className="setting-subitem">
                        <span>{t('settings_modal.backdrop_brightness_label')}</span>
                        <div className="font-scale-row backdrop-slider">
                            <SabaSlider
                                min={-50}
                                max={50}
                                step={5}
                                value={backdrop.brightness}
                                aria-label={t('settings_modal.backdrop_brightness_label')}
                                onChange={(v) => change({ brightness: v })}
                            />
                            <span className="font-scale-value">
                                {backdrop.brightness > 0 ? `+${backdrop.brightness}` : backdrop.brightness}
                            </span>
                        </div>
                    </div>
                </div>
            )}
        </div>
    );
}
