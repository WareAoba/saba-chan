import clsx from 'clsx';
import { useTranslation } from 'react-i18next';
import { ACCENT_PRESETS, type ThemeConfig, type ThemeMode } from '../../../utils/themeManager';
import CustomDropdown from '../../CustomDropdown/CustomDropdown';
import { Icon } from '../../Icon';
import { OklchPicker } from '../../ui/OklchPicker';
import { SabaSlider, SabaToggle } from '../../ui/SabaUI';
import { WindowBackdropSetting } from '../WindowBackdropSetting';

interface AppearanceTabProps {
    selectedTheme: ThemeMode;
    customTheme: ThemeConfig;
    handleThemeChange: (theme: ThemeMode) => void;
    handleCustomThemeChange: (partial: Partial<ThemeConfig>) => void;
    handleResetTheme: () => void;
}

/** 설정 › 모양: 테마·강조색·그라데이션·창 바탕·글자 크기·움직임·사이드바 */
export function AppearanceTab({
    selectedTheme,
    customTheme,
    handleThemeChange,
    handleCustomThemeChange,
    handleResetTheme,
}: AppearanceTabProps) {
    const { t } = useTranslation(['gui', 'common']);
    return (
        <section className="saba-card">
            {/* ── 1. 라이트/다크 테마 ── */}
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="palette" size="sm" /> {t('gui:settings_modal.theme_label')}
                    </span>
                    <span className="setting-description">{t('gui:settings_modal.theme_description')}</span>
                </label>
                <CustomDropdown
                    className="setting-select"
                    value={selectedTheme}
                    onChange={(val) => handleThemeChange(val)}
                    options={[
                        { value: 'auto', label: t('gui:settings_modal.theme_auto'), icon: 'monitor' },
                        { value: 'light', label: t('gui:settings_modal.theme_light'), icon: 'sun' },
                        { value: 'dark', label: t('gui:settings_modal.theme_dark'), icon: 'moon' },
                    ]}
                />
            </div>

            {/* ── 2. 강조색 프리셋 ── */}
            <div className="setting-item setting-item-vertical">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="palette" size="sm" /> {t('gui:settings_modal.accent_color_label', 'Accent Color')}
                    </span>
                    <span className="setting-description">
                        {t(
                            'gui:settings_modal.accent_color_description',
                            'Choose a brand color for buttons, highlights, and accents',
                        )}
                    </span>
                </label>
                <div className="accent-preset-grid">
                    {ACCENT_PRESETS.map((preset) => (
                        <button
                            key={preset.name}
                            className={clsx('accent-preset-btn', {
                                active: customTheme.accentColor === preset.primary,
                            })}
                            style={{
                                background: `linear-gradient(in oklch 135deg, ${preset.primary} 0%, ${preset.secondary} 100%)`,
                            }}
                            onClick={() =>
                                handleCustomThemeChange({
                                    accentColor: preset.primary,
                                    accentSecondary: preset.secondary,
                                })
                            }
                            data-tooltip={t(`gui:settings_modal.accent_${preset.name}`, preset.name)}
                            aria-label={t(`gui:settings_modal.accent_${preset.name}`, preset.name)}
                        />
                    ))}
                    <OklchPicker
                        variant="swatch"
                        className="accent-preset-btn accent-preset-custom"
                        label={t('gui:settings_modal.accent_custom', 'Custom')}
                        value={customTheme.accentColor}
                        onChange={(hex) => handleCustomThemeChange({ accentColor: hex, accentSecondary: hex })}
                    />
                </div>
            </div>

            {/* ── 2-1. 그라데이션 토글 ── */}
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="palette" size="sm" /> {t('gui:settings_modal.gradient_label', 'Gradient Effect')}
                    </span>
                    <span className="setting-description">
                        {t(
                            'gui:settings_modal.gradient_description',
                            'Apply gradient effect to buttons and app background',
                        )}
                    </span>
                </label>
                <SabaToggle
                    checked={customTheme.useGradient}
                    onChange={(checked) => handleCustomThemeChange({ useGradient: checked })}
                />
            </div>

            {/* ── 2-2. 창 바탕 반투명 ── */}
            <WindowBackdropSetting />

            {/* ── 3. 문자 크기 배율 ── */}
            <div className="setting-item setting-item-vertical">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="settings" size="sm" /> {t('gui:settings_modal.font_scale_label', 'Font Size')}
                    </span>
                    <span className="setting-description">
                        {t('gui:settings_modal.font_scale_description', 'Adjust the overall text size (50%–200%)')}
                    </span>
                </label>
                <div className="font-scale-row">
                    <SabaSlider
                        aria-label={t('gui:settings_modal.font_scale_label', 'Font Size')}
                        min={50}
                        max={200}
                        step={5}
                        value={customTheme.fontScale}
                        onChange={(v) => handleCustomThemeChange({ fontScale: v })}
                    />
                    <span className="font-scale-value">{customTheme.fontScale}%</span>
                </div>
            </div>

            {/* ── 4. 트랜지션 토글 ── */}
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="settings" size="sm" /> {t('gui:settings_modal.transitions_label', 'Animations')}
                    </span>
                    <span className="setting-description">
                        {t('gui:settings_modal.transitions_description', 'Enable or disable UI transition animations')}
                    </span>
                </label>
                <SabaToggle
                    checked={customTheme.enableTransitions}
                    onChange={(checked) => handleCustomThemeChange({ enableTransitions: checked })}
                />
            </div>

            {/* ── 사이드바 좁게 ── */}
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="server" size="sm" /> {t('gui:settings_modal.sidebar_compact_label')}
                    </span>
                    <span className="setting-description">{t('gui:settings_modal.sidebar_compact_description')}</span>
                </label>
                <SabaToggle
                    checked={!!customTheme.sidebarCompact}
                    onChange={(checked) => handleCustomThemeChange({ sidebarCompact: checked })}
                />
            </div>

            {/* ── 초기화 버튼 ── */}
            <div className="setting-item setting-item-clickable" onClick={handleResetTheme} style={{ marginTop: 8 }}>
                <label className="setting-label" style={{ cursor: 'pointer' }}>
                    <span className="setting-title">
                        <Icon name="refresh" size="sm" />{' '}
                        {t('gui:settings_modal.reset_theme_label', 'Reset to Default')}
                    </span>
                    <span className="setting-description">
                        {t('gui:settings_modal.reset_theme_description', 'Restore all appearance settings to defaults')}
                    </span>
                </label>
            </div>
        </section>
    );
}
