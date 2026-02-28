import { useRef, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import ExtensionSlot from '../ExtensionSlot';
import Icon from '../Icon';

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
    const [useContainerIsolation, setUseContainerIsolation] = useState(false);
    const nameInputRef = useRef(null);

    // 모듈별 네이티브(비-도커) 인스턴스 존재 여부 맵
    const nativeInstanceMap = {};
    for (const srv of servers) {
        if (!srv.extension_data?.docker_enabled) {
            nativeInstanceMap[srv.module] = srv.name;
        }
    }

    // 게임 카드를 눌렀을 때
    const handleGameSelect = (extName) => {
        setSelectedExtension(extName);

        // 자동 이름 생성
        const existingCount = servers.filter((s) => s.module === extName).length;
        setNewServerName(`my-${extName}-${existingCount + 1}`);

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

    useEffect(() => {
        onStepChange?.(step);
    }, [step, onStepChange]);

    // configure → select-game 으로 돌아가기
    const handleBackToSelect = () => {
        setStep('select-game');
        setSelectedExtension('');
        setNewServerName('');
        setUseContainerIsolation(false);
    };

    const handleSubmit = async () => {
        if (!newServerName.trim()) return;
        if (!selectedExtension) return;

        setSubmitting(true);
        try {
            await onAddServer({
                name: newServerName.trim(),
                module_name: selectedExtension,
                accept_eula: true,
                use_container: useContainerIsolation,
            });
        } finally {
            setSubmitting(false);
        }

        // 폼 초기화
        setNewServerName('');
        setSelectedExtension('');
        setUseContainerIsolation(false);
        setStep('select-game');
    };

    const canSubmit = newServerName.trim() && selectedExtension && !submitting
        && !(nativeInstanceMap[selectedExtension] && !useContainerIsolation);

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
                                        const hasNative = !!nativeInstanceMap[m.name];
                                        return (
                                            <button
                                                key={m.name}
                                                className={`as-game-card${hasNative ? ' as-game-card--has-native' : ''}`}
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
                                                {hasNative && (
                                                    <span className="as-game-card-badge">
                                                        <Icon name="check" size="xs" /> {t('add_server_modal.native_exists')}
                                                    </span>
                                                )}
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

                    {/* 익스텐션 슬롯 — 컨테이너 격리 토글 등 */}
                    <ExtensionSlot
                        slotId="AddServer.options"
                        options={{ use_container: useContainerIsolation }}
                        onOptionsChange={(opts) => {
                            if (opts.use_container !== undefined) setUseContainerIsolation(opts.use_container);
                        }}
                        t={t}
                    />

                    {/* 네이티브 인스턴스 중복 경고 */}
                    {nativeInstanceMap[selectedExtension] && !useContainerIsolation && (
                        <p className="as-native-limit-warning">
                            <Icon name="alertTriangle" size="sm" />
                            {t('add_server_modal.native_limit_warning', {
                                existing: nativeInstanceMap[selectedExtension],
                                defaultValue: `A native instance '{{existing}}' already exists for this module. Enable container isolation to create another instance.`,
                            })}
                        </p>
                    )}

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
