import { useRef, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AgreementList, allChecked } from '../AgreementList';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { AddServerOptions, addServerExtensionData } from '../DataSlots';
import ExtensionSlot from '../ExtensionSlot';
import { useExtensions } from '../../contexts/ExtensionContext';
import Icon from '../Icon';
import { Segmented } from '../ui/StatusChip';
import { TrustCreateNotice } from '../ui/TrustBadge';

/** 모듈이 만들 때 고르라고 한 설정 (`install = true`, 예: 마인크래프트 서버 종류) */
const installFields = (module) => (module?.settings?.fields || []).filter((f) => f.install && f.field_type === 'select');

/**
 * AddInstanceNewServer — "새 서버로 추가" 흐름.
 *
 * 2단계 구성:
 *   step 'select-game'  — 아이콘+이름 카드 그리드에서 게임 모듈 선택
 *   step 'configure'    — 인스턴스 이름 입력 + 옵션 + 제출
 */
export function AddInstanceNewServer({
    extensions,
    servers,
    onRefreshextensions,
    onAddServer,
    onStepChange,
    onBack,
    onClose,
}) {
    const { t } = useTranslation('gui');
    const [step, setStep] = useState('select-game'); // 'select-game' | 'configure'
    const [selectedExtension, setSelectedExtension] = useState('');
    const [newServerName, setNewServerName] = useState('');
    const [submitting, setSubmitting] = useState(false);
    const [installValues, setInstallValues] = useState({});
    const [preset, setPreset] = useState('');
    const [agreed, setAgreed] = useState({}); // 약관 id → 체크 여부
    const [extValues, setExtValues] = useState({}); // AddServer.options 답 (field → 값)
    const { extensions: daemonExtensions } = useExtensions();
    const nameInputRef = useRef(null);

    // 게임 카드를 눌렀을 때
    const handleGameSelect = (extName) => {
        onStepChange?.('configure'); // 높이 캡처 후 부모 상태 변경 (배칭)
        setSelectedExtension(extName);

        // 자동 이름 생성
        const existingCount = servers.filter((s) => s.module === extName).length;
        setNewServerName(`my-${extName}-${existingCount + 1}`);
        setInstallValues({});
        setPreset('');
        setAgreed({});
        setExtValues({});

        setStep('configure');
    };

    // configure 단계 진입 시 이름 인풋에 포커스
    useEffect(() => {
        if (step === 'configure' && nameInputRef.current) {
            // 짧은 딜레이로 애니메이션 후 포커스
            const timer = setTimeout(() => nameInputRef.current?.focus(), 80);
            return () => clearTimeout(timer);
        }
    }, [step]);

    // onStepChange는 handleGameSelect/handleBackToSelect에서 동기적으로 호출
    // (useEffect로 호출하면 paint 이후라 높이 애니메이션 캡처가 실패함)

    // configure → select-game 으로 돌아가기
    const handleBackToSelect = () => {
        onStepChange?.('select-game'); // 높이 캡처 후 부모 상태 변경 (배칭)
        setStep('select-game');
        setSelectedExtension('');
        setNewServerName('');
    };

    const handleSubmit = async () => {
        if (!canSubmit) return;

        setSubmitting(true);
        const extensionData = addServerExtensionData(daemonExtensions, extValues);
        let created;
        try {
            created = await onAddServer({
                name: newServerName.trim(),
                module_name: selectedExtension,
                module_settings: installValues,
                // 사용자가 체크한 약관만 (module.toml [[agreements]])
                ...(agreements.length ? { accept_agreements: agreements.map((a) => a.id) } : {}),
                // 프리셋 값은 데몬이 푼다 (I1-b)
                ...(preset ? { preset } : {}),
                // 익스텐션이 물은 칸 (AddServer.options)
                ...(extensionData ? { extension_data: extensionData } : {}),
            });
        } finally {
            setSubmitting(false);
        }
        // 만들지 못했으면 입력을 남겨 고쳐서 다시 보낼 수 있게 한다
        if (created === false) return;

        // 폼 초기화
        setNewServerName('');
        setSelectedExtension('');
        setStep('select-game');
    };

    const agreementModule = extensions.find((m) => m.name === selectedExtension);
    const agreements = agreementModule?.agreements || [];
    const canSubmit = newServerName.trim() && selectedExtension && !submitting && allChecked(agreements, agreed);

    // ──────────────────────────────────────────
    //  Step 1: 게임 선택 (아이콘 카드 그리드)
    // ──────────────────────────────────────────
    if (step === 'select-game') {
        return (
            <div className="add-instance-stage" key="new-select-game">
                <div className="modal-header add-server-header">
                    <div className="as-header-row">
                        <button className="ai-back-btn" type="button" onClick={onBack}>
                            <Icon name="chevronLeft" size="sm" />
                        </button>
                        <div className="as-header-copy">
                            <h3>{t('add_server_modal.title')}</h3>
                            <p className="add-server-subtitle">{t('add_server_modal.select_game_subtitle')}</p>
                        </div>
                    </div>
                </div>

                <div className="modal-body add-server-body">
                    <div className="as-body-stack">
                        {extensions.length === 0 ? (
                            <p className="as-empty-hint">
                                <Icon name="alertCircle" size="sm" />
                                {t('add_server_modal.no_extensions')}
                            </p>
                        ) : (
                            <div className="as-grid-wrap">
                                <div className="as-game-grid">
                                    {extensions.map((m) => {
                                        const displayName = t(`mod_${m.name}:module.display_name`, {
                                            defaultValue: m.game_name || m.name,
                                        });
                                        return (
                                            <button
                                                key={m.name}
                                                className="as-game-card"
                                                type="button"
                                                onClick={() => handleGameSelect(m.name)}
                                            >
                                                <div className="as-game-card-icon">
                                                    {m.icon ? (
                                                        <img src={m.icon} alt={displayName} />
                                                    ) : (
                                                        <div className="as-game-card-icon-placeholder">
                                                            <Icon name="gamepad" size="lg" />
                                                        </div>
                                                    )}
                                                </div>
                                                <span className="as-game-card-name">{displayName}</span>
                                                <span className="as-game-card-version">v{m.version}</span>
                                            </button>
                                        );
                                    })}
                                </div>
                            </div>
                        )}
                    </div>
                </div>

                <div className="modal-footer add-server-footer">
                    <button className="btn btn-cancel" onClick={onClose}>
                        {t('modals.cancel')}
                    </button>
                </div>
            </div>
        );
    }

    // ──────────────────────────────────────────
    //  Step 2: 인스턴스 구성 (이름 + 옵션)
    // ──────────────────────────────────────────
    const selectedModule = extensions.find((m) => m.name === selectedExtension);
    const selectedDisplayName = selectedModule
        ? t(`mod_${selectedModule.name}:module.display_name`, {
              defaultValue: selectedModule.game_name || selectedModule.name,
          })
        : selectedExtension;

    return (
        <div className="add-instance-stage" key="new-configure">
            {/* ── Header ── */}
            <div className="modal-header add-server-header">
                <div className="as-header-row">
                    <button className="ai-back-btn" type="button" onClick={handleBackToSelect} disabled={submitting}>
                        <Icon name="chevronLeft" size="sm" />
                    </button>
                    <div className="as-header-copy">
                        <h3>{t('add_server_modal.configure_title')}</h3>
                        <p className="add-server-subtitle">
                            {t('add_server_modal.configure_subtitle')}
                        </p>
                    </div>
                </div>
            </div>

            {/* ── Body ── */}
            <div className="modal-body add-server-body">
                <div className="as-body-stack">
                    {/* 선택된 게임 요약 */}
                    <div className="as-selected-game">
                        <div className="as-selected-game-icon">
                            {selectedModule?.icon ? (
                                <img src={selectedModule.icon} alt={selectedDisplayName} />
                            ) : (
                                <div className="as-game-card-icon-placeholder">
                                    <Icon name="gamepad" size="md" />
                                </div>
                            )}
                        </div>
                        <div className="as-selected-game-info">
                            <span className="as-selected-game-name">{selectedDisplayName}</span>
                            <span className="as-selected-game-version">v{selectedModule?.version}</span>
                        </div>
                        <button
                            className="as-change-game-btn"
                            type="button"
                            onClick={handleBackToSelect}
                            disabled={submitting}
                        >
                            {t('add_server_modal.change_game')}
                        </button>
                    </div>

                    {/* 확인 안 된·바뀐 모듈: 게임 서버는 격리되지 않는다 */}
                    <TrustCreateNotice trust={selectedModule?.trust} />

                    {/* 인스턴스 이름 */}
                    <div className="as-section as-section-card">
                        <label className="as-label">
                            <Icon name="server" size="sm" />
                            {t('add_server_modal.server_name')}
                        </label>
                        <input
                            ref={nameInputRef}
                            className="as-input"
                            type="text"
                            placeholder={t('add_server_modal.server_name_placeholder')}
                            value={newServerName}
                            onChange={(e) => setNewServerName(e.target.value)}
                            disabled={submitting}
                            onKeyDown={(e) => {
                                if (e.key === 'Enter' && canSubmit) handleSubmit();
                            }}
                        />
                    </div>

                    {/* 모듈이 만들 때 고르라고 한 설정 (설치할 서버 파일이 달라진다) */}
                    {installFields(selectedModule).map((field) => {
                        const modNs = `mod_${selectedModule.name}`;
                        return (
                            <div className="as-section as-section-card" key={field.name}>
                                <label className="as-label">
                                    <Icon name="settings" size="sm" />
                                    {t(`${modNs}:settings.${field.name}.label`, { defaultValue: field.label })}
                                </label>
                                <CustomDropdown
                                    value={String(installValues[field.name] ?? field.default ?? '')}
                                    onChange={(val) => setInstallValues((prev) => ({ ...prev, [field.name]: val }))}
                                    options={(field.options || []).map((opt) => ({ value: opt, label: opt }))}
                                    disabled={submitting}
                                />
                                <p className="as-provision-hint">
                                    {t(`${modNs}:settings.${field.name}.description`, {
                                        defaultValue: field.description || '',
                                    })}
                                </p>
                            </div>
                        );
                    })}

                    {/* 모듈 추천 프리셋 (module.toml [[presets]]) */}
                    {selectedModule?.presets?.length > 0 && (
                        <div className="as-section as-section-card">
                            <span className="as-label">
                                <Icon name="lightbulb" size="sm" />
                                {t('add_server_modal.preset_title')}
                            </span>
                            <Segmented
                                ariaLabel={t('add_server_modal.preset_title')}
                                value={preset}
                                onChange={setPreset}
                                items={[
                                    { id: '', label: t('add_server_modal.preset_none') },
                                    ...selectedModule.presets.map((p) => ({
                                        id: p.id,
                                        label: t(`mod_${selectedModule.name}:presets.${p.id}.label`, {
                                            defaultValue: p.label || p.id,
                                        }),
                                    })),
                                ]}
                            />
                            <p className="as-provision-hint">
                                {preset
                                    ? t(`mod_${selectedModule.name}:presets.${preset}.description`, {
                                          defaultValue:
                                              selectedModule.presets.find((p) => p.id === preset)?.description || '',
                                      })
                                    : t('add_server_modal.preset_none_hint')}
                            </p>
                        </div>
                    )}

                    {/* 모듈이 선언한 약관: 모두 동의해야 만들 수 있다 */}
                    {agreements.length > 0 && (
                        <div className="as-section as-section-card">
                            <span className="as-label">
                                <Icon name="file" size="sm" />
                                {t('agreements.title')}
                            </span>
                            <AgreementList
                                module={selectedExtension}
                                agreements={agreements}
                                checked={agreed}
                                onToggle={(id, on) => setAgreed((prev) => ({ ...prev, [id]: on }))}
                                disabled={submitting}
                            />
                        </div>
                    )}

                    {/* 익스텐션 슬롯 — 서버 생성 옵션 */}
                    <ExtensionSlot slotId="AddServer.options" t={t} />
                    <AddServerOptions
                        values={extValues}
                        onChange={(field, value) => setExtValues((prev) => ({ ...prev, [field]: value }))}
                        disabled={submitting}
                    />

                    {/* 프로비저닝 안내 */}
                    <p className="as-provision-hint">
                        <Icon name="info" size="sm" />
                        {t('add_server_modal.provision_hint')}
                    </p>
                </div>
            </div>

            {/* ── Footer ── */}
            <div className="modal-footer add-server-footer">
                <button className="btn btn-cancel" onClick={onClose} disabled={submitting}>
                    {t('modals.cancel')}
                </button>
                <button className="btn btn-confirm" onClick={handleSubmit} disabled={!canSubmit}>
                    {submitting ? (
                        <>
                            <Icon name="refresh" size="sm" className="spin" /> {t('add_server_modal.provisioning')}
                        </>
                    ) : (
                        <>
                            <Icon name="plus" size="sm" /> {t('add_server_modal.add_server')}
                        </>
                    )}
                </button>
            </div>
        </div>
    );
}
