import { useTranslation } from 'react-i18next';
import { type SlotEntry, useExtensions } from '../../contexts/ExtensionContext';
import type { Json } from '../../types/daemon';
import { ExtensionConfigForm } from '../ExtensionConfigForm';
import { SlotView } from '../ExtensionSlot';

/**
 * 설정 창의 익스텐션 탭 (`Settings.tab`). 컴포넌트가 있으면 그것을, 없으면 manifest `config_fields` 자동 폼을 그린다.
 * 컴포넌트는 getConfig·setConfig로 같은 설정(GET/PUT /api/extensions/:id/config)을 읽고 쓴다
 */
export function ExtensionSettingsPanel({ entry }: { entry: SlotEntry }) {
    const { t } = useTranslation('gui');
    const { extensions } = useExtensions();
    const ext = extensions.find((e) => e.id === entry.ext);
    if (!ext) return null;
    const empty = <p className="setting-description">{t('extension_config.none')}</p>;
    const form = Object.keys(ext.config_fields || {}).length ? <ExtensionConfigForm ext={ext} inline /> : empty;
    return (
        <SlotView
            entry={entry}
            slotId="Settings.tab"
            fallback={form}
            t={t}
            getConfig={() => window.api.extensionConfigGet(ext.id)}
            setConfig={(config: Record<string, Json>) => window.api.extensionConfigSet(ext.id, config)}
        />
    );
}
