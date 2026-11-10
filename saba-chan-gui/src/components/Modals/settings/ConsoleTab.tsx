import clsx from 'clsx';
import { useTranslation } from 'react-i18next';
import { useSettingsStore } from '../../../stores/useSettingsStore';
import {
    CONSOLE_FONT_CHOICES,
    CONSOLE_FONT_NAMES,
    CONSOLE_FONT_SAMPLE,
    CONSOLE_FONT_SLOTS,
} from '../../../utils/consoleFonts';
import { TOKENS } from '../../../utils/syntaxHighlight';
import type { ThemeConfig } from '../../../utils/themeManager';
import CustomDropdown from '../../CustomDropdown/CustomDropdown';
import { Icon } from '../../Icon';
import { OklchPicker } from '../../ui/OklchPicker';
import { SabaSlider, SabaToggle } from '../../ui/SabaUI';
import { clearedTokenColors, setOption } from './options';

/** 지금 테마가 정한 토큰 기본색 (사용자 색을 걷어낸 값) */
function tokenDefaultColor(token: string) {
    const root = document.documentElement;
    const name = `--tok-${token}`;
    const custom = root.style.getPropertyValue(name);
    if (custom) root.style.removeProperty(name);
    const value = getComputedStyle(root).getPropertyValue(name).trim();
    if (custom) root.style.setProperty(name, custom);
    return value || '#cdd6f4';
}

interface ConsoleTabProps {
    customTheme: ThemeConfig;
    handleCustomThemeChange: (partial: Partial<ThemeConfig>) => void;
}

/** 설정 › 콘솔: 구문 강조·시각·줄 접기·색·글꼴·글자 크기·버퍼 */
export function ConsoleTab({ customTheme, handleCustomThemeChange }: ConsoleTabProps) {
    const { t } = useTranslation(['gui', 'common']);
    const consoleBufferSize = useSettingsStore((s) => s.consoleBufferSize) || 2000;
    const consoleTimestamps = useSettingsStore((s) => s.consoleTimestamps);
    const consoleFonts = useSettingsStore((s) => s.consoleFonts);
    const consoleTimestampDate = useSettingsStore((s) => s.consoleTimestampDate);
    const consoleWrap = useSettingsStore((s) => s.consoleWrap);
    const consoleAutoScroll = useSettingsStore((s) => s.consoleAutoScroll);
    return (
        <section className="saba-card">
            {/* ── 5-1. 콘솔 구문강조 토글 ── */}
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="terminal" size="sm" />{' '}
                        {t('gui:settings_modal.console_syntax_label', 'Syntax Highlighting')}
                    </span>
                    <span className="setting-description">
                        {t(
                            'gui:settings_modal.console_syntax_description',
                            'Color-code log output in the server console',
                        )}
                    </span>
                </label>
                <SabaToggle
                    checked={customTheme.consoleSyntaxHighlight}
                    onChange={(checked) => handleCustomThemeChange({ consoleSyntaxHighlight: checked })}
                />
            </div>

            {/* ── 5-1b. 구문 강조 색 (토큰마다, 빈 값이면 테마 기본색) ── */}
            {customTheme.consoleSyntaxHighlight && (
                <div className="setting-item setting-item-vertical">
                    <div className="setting-label-row">
                        <label className="setting-label">
                            <span className="setting-title">
                                <Icon name="palette" size="sm" /> {t('gui:settings_modal.console_token_colors_label')}
                            </span>
                            <span className="setting-description">
                                {t('gui:settings_modal.console_token_colors_description')}
                            </span>
                        </label>
                        <button
                            type="button"
                            className="btn btn-ghost btn-sm"
                            disabled={!Object.values(customTheme.consoleTokenColors || {}).some(Boolean)}
                            onClick={() => handleCustomThemeChange({ consoleTokenColors: clearedTokenColors() })}
                        >
                            <Icon name="undo" size="sm" /> {t('gui:settings_modal.console_token_colors_reset')}
                        </button>
                    </div>
                    <div className="console-token-grid">
                        {TOKENS.map((token) => {
                            const custom = customTheme.consoleTokenColors?.[token] || '';
                            const label = t(`gui:settings_modal.console_token_${token}`);
                            return (
                                <div key={token} className={clsx('console-token-row', { custom })}>
                                    <span className={`console-token-name tok-${token}`}>{label}</span>
                                    <OklchPicker
                                        label={label}
                                        value={custom || tokenDefaultColor(token)}
                                        onChange={(hex) =>
                                            handleCustomThemeChange({
                                                consoleTokenColors: { ...customTheme.consoleTokenColors, [token]: hex },
                                            })
                                        }
                                    />
                                </div>
                            );
                        })}
                    </div>
                </div>
            )}

            {/* 줄 앞 시각 (데몬이 받은 시각) */}
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="clock" size="sm" /> {t('gui:settings_modal.console_timestamps_label')}
                    </span>
                    <span className="setting-description">
                        {t('gui:settings_modal.console_timestamps_description')}
                    </span>
                </label>
                <CustomDropdown
                    className="setting-select"
                    value={consoleTimestamps}
                    onChange={(val) => setOption({ consoleTimestamps: val })}
                    options={['off', '24h', '12h'].map((value) => ({
                        value,
                        label: t(`gui:settings_modal.console_timestamps_${value}`),
                    }))}
                />
            </div>
            {consoleTimestamps !== 'off' && (
                <div className="setting-item">
                    <label className="setting-label">
                        <span className="setting-title">
                            <Icon name="clock" size="sm" /> {t('gui:settings_modal.console_timestamp_date_label')}
                        </span>
                    </label>
                    <SabaToggle
                        checked={consoleTimestampDate}
                        onChange={(checked) => setOption({ consoleTimestampDate: checked })}
                    />
                </div>
            )}

            {/* ── 긴 줄 접기 · 새 줄 따라가기 ── */}
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="terminal" size="sm" /> {t('gui:settings_modal.console_wrap_label')}
                    </span>
                    <span className="setting-description">{t('gui:settings_modal.console_wrap_description')}</span>
                </label>
                <SabaToggle checked={consoleWrap} onChange={(checked) => setOption({ consoleWrap: checked })} />
            </div>
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="terminal" size="sm" /> {t('gui:settings_modal.console_auto_scroll_label')}
                    </span>
                    <span className="setting-description">
                        {t('gui:settings_modal.console_auto_scroll_description')}
                    </span>
                </label>
                <SabaToggle
                    checked={consoleAutoScroll}
                    onChange={(checked) => setOption({ consoleAutoScroll: checked })}
                />
            </div>

            {/* ── 5-2. 콘솔 뒷배경색 ── */}
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="terminal" size="sm" />{' '}
                        {t('gui:settings_modal.console_bg_label', 'Console Background')}
                    </span>
                    <span className="setting-description">
                        {t('gui:settings_modal.console_bg_description', 'Background color of the console panel')}
                    </span>
                </label>
                <OklchPicker
                    label={t('gui:settings_modal.console_bg_label', 'Console Background')}
                    value={customTheme.consoleBgColor}
                    onChange={(hex) => handleCustomThemeChange({ consoleBgColor: hex })}
                />
            </div>

            {/* ── 5-3. 콘솔 기본 폰트 컬러 ── */}
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="terminal" size="sm" />{' '}
                        {t('gui:settings_modal.console_text_label', 'Console Text Color')}
                    </span>
                    <span className="setting-description">
                        {t('gui:settings_modal.console_text_description', 'Default text color in the console output')}
                    </span>
                </label>
                <OklchPicker
                    label={t('gui:settings_modal.console_text_label', 'Console Text Color')}
                    value={customTheme.consoleTextColor}
                    onChange={(hex) => handleCustomThemeChange({ consoleTextColor: hex })}
                />
            </div>

            {/* 콘솔 글꼴: 글자마다 하나 (utils/consoleFonts.js) */}
            <div className="setting-item setting-item-vertical">
                <div className="setting-label">
                    <span className="setting-title">
                        <Icon name="terminal" size="sm" /> {t('gui:settings_modal.console_fonts_label')}
                    </span>
                    <span className="setting-description">{t('gui:settings_modal.console_fonts_description')}</span>
                </div>
                <div className="console-font-grid">
                    {CONSOLE_FONT_SLOTS.map((slot) => (
                        <div key={slot} className="console-font-slot">
                            <span className="console-font-slot-label">
                                {t(`gui:settings_modal.console_fonts_${slot}`)}
                            </span>
                            <CustomDropdown
                                className="setting-select"
                                ariaLabel={t(`gui:settings_modal.console_fonts_${slot}`)}
                                value={consoleFonts[slot]}
                                onChange={(val) => setOption({ consoleFonts: { ...consoleFonts, [slot]: val } })}
                                options={CONSOLE_FONT_CHOICES[slot].map((value) => ({
                                    value,
                                    label: CONSOLE_FONT_NAMES[value as keyof typeof CONSOLE_FONT_NAMES],
                                }))}
                            />
                        </div>
                    ))}
                </div>
            </div>

            {/* ── 5-4. 콘솔 텍스트 크기 ── */}
            <div className="setting-item setting-item-vertical">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="terminal" size="sm" />{' '}
                        {t('gui:settings_modal.console_font_scale_label', 'Console Text Size')}
                    </span>
                    <span className="setting-description">
                        {t(
                            'gui:settings_modal.console_font_scale_description',
                            'Adjust the console text size (50%–200%)',
                        )}
                    </span>
                </label>
                <div className="font-scale-row">
                    <SabaSlider
                        aria-label={t('gui:settings_modal.console_font_scale_label', 'Console Text Size')}
                        min={50}
                        max={200}
                        step={5}
                        value={customTheme.consoleFontScale}
                        onChange={(v) => handleCustomThemeChange({ consoleFontScale: v })}
                    />
                    <span className="font-scale-value">{customTheme.consoleFontScale}%</span>
                </div>
            </div>

            {/* ── 콘솔 미리보기 ── */}
            <div
                className="console-preview-box"
                style={{
                    background: customTheme.consoleBgColor,
                    color: customTheme.consoleTextColor,
                    fontSize: `calc(12.5px * ${customTheme.consoleFontScale / 100})`,
                }}
            >
                {(() => {
                    // 하이라이트가 켜져 있으면 실제 콘솔과 같은 토큰 색으로 (테마 --tok-*)
                    const tok = (token: string, text: string) =>
                        customTheme.consoleSyntaxHighlight ? <span className={`tok-${token}`}>{text}</span> : text;
                    return (
                        <>
                            <div className="console-preview-line">
                                {tok('timestamp', '[12:34:56]')} Server started on port {tok('number', '25565')}
                            </div>
                            {/* 글꼴 미리보기: 영문·한글·일본어·한자 한 줄 (글자마다 고른 글꼴) */}
                            <div className="console-preview-line">
                                {tok('timestamp', '[12:35:02]')} FPS {tok('number', '59')} {CONSOLE_FONT_SAMPLE}
                            </div>
                            <div className="console-preview-line">
                                {tok('timestamp', '[12:34:57]')} {tok('player', 'Steve')} joined the game
                            </div>
                            <div className="console-preview-line">
                                {tok('timestamp', '[12:34:58]')} {tok('warn', 'WARN')}: Low memory detected
                            </div>
                            <div className="console-preview-line">
                                {tok('timestamp', '[12:34:59]')} {tok('error', 'ERROR')}: Connection timeout
                            </div>
                        </>
                    );
                })()}
            </div>

            {/* 콘솔 버퍼 크기 설정 */}
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="terminal" size="sm" /> {t('gui:settings_modal.console_buffer_label')}
                    </span>
                    <span className="setting-description">{t('gui:settings_modal.console_buffer_description')}</span>
                </label>
                <CustomDropdown
                    className="setting-select"
                    value={consoleBufferSize}
                    onChange={(val) => setOption({ consoleBufferSize: Number(val) })}
                    options={[
                        { value: 500, label: '500' },
                        { value: 1000, label: '1,000' },
                        { value: 2000, label: '2,000' },
                        { value: 5000, label: '5,000' },
                        { value: 10000, label: '10,000' },
                    ]}
                />
            </div>
        </section>
    );
}
