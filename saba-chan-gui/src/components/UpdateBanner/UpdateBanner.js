import React, { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import './UpdateBanner.css';

/**
 * 업데이트 알림 배너
 * - 백그라운드 업데이트 체크 결과 표시
 * - 클릭 시 인앱 업데이트 모달 오픈 콜백
 * - 업데이트 완료 시 내역(컴포넌트별 버전 변경) 표시
 */
function UpdateBanner({ onOpenUpdateModal }) {
    const { t } = useTranslation('gui');
    
    const [visible, setVisible] = useState(false);
    const [bannerType, setBannerType] = useState('available'); // 'available' | 'completed' | 'downloading'
    const [updateInfo, setUpdateInfo] = useState(null);
    const [dismissed, setDismissed] = useState(false);
    const [isClosing, setIsClosing] = useState(false);
    const [expanded, setExpanded] = useState(false); // 내역 확장

    // 업데이트 가능 알림 수신
    useEffect(() => {
        if (!window.api?.onUpdatesAvailable) return;

        const handleUpdates = (data) => {
            console.log('[UpdateBanner] Updates available:', data);
            if (data.updates_available > 0 && !dismissed) {
                setUpdateInfo({
                    count: data.updates_available,
                    names: data.update_names || [],
                    components: data.components || [],
                    details: data.details || [],
                });
                setBannerType('available');
                setVisible(true);
                setExpanded(false);
            }
        };

        window.api.onUpdatesAvailable(handleUpdates);
        return () => window.api.offUpdatesAvailable?.();
    }, [dismissed]);

    // 업데이트 완료 알림 수신
    useEffect(() => {
        if (!window.api?.onUpdateCompleted) return;

        const handleCompleted = (data) => {
            console.log('[UpdateBanner] Update completed:', data);
            setUpdateInfo({
                count: data.updated_count || 0,
                names: data.updated_names || [],
                success: data.success,
                details: data.details || [],
            });
            setBannerType('completed');
            setVisible(true);
            setDismissed(false);
            setExpanded(false);

            setTimeout(() => {
                handleClose();
            }, 15000);
        };

        window.api.onUpdateCompleted(handleCompleted);
        return () => window.api.offUpdateCompleted?.();
    }, []);

    // 배너 닫기
    const handleClose = useCallback(() => {
        setIsClosing(true);
        setTimeout(() => {
            setVisible(false);
            setIsClosing(false);
            setExpanded(false);
            if (bannerType === 'available') {
                setDismissed(true);
            }
        }, 300);
    }, [bannerType]);

    // 인앱 업데이트 모달 열기
    const handleOpenModal = useCallback(() => {
        if (onOpenUpdateModal) {
            onOpenUpdateModal();
        }
        handleClose();
    }, [onOpenUpdateModal, handleClose]);

    // 배너 클릭
    const handleBannerClick = useCallback(() => {
        if (bannerType === 'available') {
            handleOpenModal();
        } else if (bannerType === 'completed') {
            setExpanded(prev => !prev);
        }
    }, [bannerType, handleOpenModal]);

    // 수동 새로고침
    const handleRefresh = useCallback(async () => {
        setDismissed(false);
        setBannerType('downloading');
        setVisible(true);
        setExpanded(false);

        try {
            const result = await window.api?.updaterCheck?.();
            if (result?.ok && result?.updates_available > 0) {
                setUpdateInfo({
                    count: result.updates_available,
                    names: result.update_names || [],
                    components: result.components || [],
                    details: [],
                });
                setBannerType('available');
            } else {
                handleClose();
            }
        } catch (err) {
            console.error('[UpdateBanner] Check failed:', err);
            handleClose();
        }
    }, [handleClose]);

    // window.showUpdateBanner 전역 함수 등록
    useEffect(() => {
        window.showUpdateBanner = (type, info) => {
            setUpdateInfo(info);
            setBannerType(type);
            setVisible(true);
            setDismissed(false);
            setExpanded(false);
        };

        window.checkForUpdates = handleRefresh;

        return () => {
            delete window.showUpdateBanner;
            delete window.checkForUpdates;
        };
    }, [handleRefresh]);

    if (!visible) return null;

    // 배너 내용
    let icon, title, message, actionText;
    switch (bannerType) {
        case 'available':
            icon = 'download';
            title = t('updates.available_title', '업데이트 사용 가능');
            message = updateInfo?.names?.length > 0
                ? t('updates.available_message', { count: updateInfo.count, names: updateInfo.names.join(', ') })
                : t('updates.available_count', { count: updateInfo?.count || 0 });
            actionText = t('updates.btn_check', '업데이트 확인');
            break;
        case 'completed':
            icon = 'checkCircle';
            title = t('updates.completed_title', '업데이트 완료');
            message = updateInfo?.success
                ? t('updates.completed_success', '모든 업데이트가 성공적으로 적용되었습니다.')
                : t('updates.completed_partial', '일부 업데이트가 적용되었습니다.');
            actionText = null;
            break;
        case 'downloading':
            icon = 'refresh';
            title = t('updates.checking_title', '업데이트 확인 중');
            message = t('updates.checking_message', '새로운 업데이트가 있는지 확인하고 있습니다...');
            actionText = null;
            break;
        default:
            return null;
    }

    const details = updateInfo?.details || [];

    return (
        <div className={`update-banner ${bannerType} ${isClosing ? 'closing' : ''} ${expanded ? 'expanded' : ''}`}>
            <div className="update-banner-content" onClick={handleBannerClick}>
                <div className="update-banner-icon">
                    <Icon name={icon} size="sm" />
                </div>
                <div className="update-banner-text">
                    <span className="update-banner-title">{title}</span>
                    <span className="update-banner-message">{message}</span>
                </div>
                {actionText && (
                    <button className="update-banner-action" onClick={(e) => { e.stopPropagation(); handleOpenModal(); }}>
                        {actionText}
                    </button>
                )}
                {bannerType === 'completed' && details.length > 0 && (
                    <button className="update-banner-expand" onClick={(e) => { e.stopPropagation(); setExpanded(prev => !prev); }}>
                        <Icon name={expanded ? 'chevronUp' : 'chevronDown'} size="xs" />
                    </button>
                )}
            </div>
            {/* 업데이트 내역 확장 영역 */}
            {expanded && details.length > 0 && (
                <div className="update-banner-details">
                    {details.map((d, i) => (
                        <div key={i} className="update-banner-detail-row">
                            <span className="update-banner-detail-name">{d.name}</span>
                            <span className="update-banner-detail-ver">
                                v{d.from} <span className="update-banner-detail-arrow">→</span> <strong>v{d.to}</strong>
                            </span>
                        </div>
                    ))}
                </div>
            )}
            <button className="update-banner-close" onClick={(e) => { e.stopPropagation(); handleClose(); }}>
                <Icon name="x" size="xs" />
            </button>
        </div>
    );
}

export default UpdateBanner;
