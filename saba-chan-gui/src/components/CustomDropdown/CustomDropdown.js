import React, { useState, useRef, useEffect, useCallback } from 'react';
import ReactDOM from 'react-dom';
import { Icon } from '../Icon';
import './CustomDropdown.css';

/**
 * 커스텀 드롭다운 컴포넌트 (네이티브 select 대체)
 * Portal을 사용하여 메뉴가 모달 DOM을 벗어나서도 렌더링됨
 */
function CustomDropdown({ value, onChange, options = [], placeholder, className, disabled }) {
    const [isOpen, setIsOpen] = useState(false);
    const [menuStyle, setMenuStyle] = useState({});
    const triggerRef = useRef(null);
    const menuRef = useRef(null);

    // 트리거 위치 기반으로 메뉴 좌표 계산
    const updateMenuPosition = useCallback(() => {
        if (!triggerRef.current) return;
        const rect = triggerRef.current.getBoundingClientRect();
        setMenuStyle({
            position: 'fixed',
            top: rect.bottom + 4,
            left: rect.left,
            minWidth: rect.width,
            zIndex: 10000,
        });
    }, []);

    // 열릴 때 위치 계산 + 스크롤/리사이즈 추적
    useEffect(() => {
        if (!isOpen) return;
        updateMenuPosition();
        window.addEventListener('scroll', updateMenuPosition, true);
        window.addEventListener('resize', updateMenuPosition);
        return () => {
            window.removeEventListener('scroll', updateMenuPosition, true);
            window.removeEventListener('resize', updateMenuPosition);
        };
    }, [isOpen, updateMenuPosition]);

    // 외부 클릭 시 닫기 (트리거 + 포탈 메뉴 둘 다 체크)
    useEffect(() => {
        const handleClickOutside = (e) => {
            if (
                triggerRef.current && !triggerRef.current.contains(e.target) &&
                menuRef.current && !menuRef.current.contains(e.target)
            ) {
                setIsOpen(false);
            }
        };
        if (isOpen) {
            document.addEventListener('mousedown', handleClickOutside);
            return () => document.removeEventListener('mousedown', handleClickOutside);
        }
    }, [isOpen]);

    // ESC 키로 닫기
    useEffect(() => {
        const handleKeyDown = (e) => {
            if (e.key === 'Escape') setIsOpen(false);
        };
        if (isOpen) {
            document.addEventListener('keydown', handleKeyDown);
            return () => document.removeEventListener('keydown', handleKeyDown);
        }
    }, [isOpen]);

    const selectedOption = options.find(o => String(o.value) === String(value));

    const handleSelect = (optionValue) => {
        onChange(optionValue);
        setIsOpen(false);
    };

    const handleToggle = () => {
        if (disabled) return;
        setIsOpen(prev => !prev);
    };

    // Portal로 body에 렌더링되는 메뉴
    const menu = isOpen
        ? ReactDOM.createPortal(
            <div className="custom-dropdown-menu" ref={menuRef} style={menuStyle}>
                <div className="custom-dropdown-scroll">
                    {options.map((option) => {
                        const isSelected = String(option.value) === String(value);
                        return (
                            <div
                                key={option.value}
                                className={`custom-dropdown-item ${isSelected ? 'selected' : ''}`}
                                onClick={() => handleSelect(option.value)}
                            >
                                <span className="custom-dropdown-item-content">
                                    {option.icon && <Icon name={option.icon} size="sm" />}
                                    <span>{option.label}</span>
                                </span>
                                {isSelected && (
                                    <span className="custom-dropdown-check">
                                        <Icon name="check" size="sm" />
                                    </span>
                                )}
                            </div>
                        );
                    })}
                </div>
            </div>,
            document.body
        )
        : null;

    return (
        <div className={`custom-dropdown ${className || ''} ${isOpen ? 'open' : ''} ${disabled ? 'disabled' : ''}`}>
            <button
                className="custom-dropdown-trigger"
                onClick={handleToggle}
                ref={triggerRef}
                type="button"
                tabIndex={0}
            >
                <span className="custom-dropdown-value">
                    {selectedOption ? (
                        <>
                            {selectedOption.icon && <Icon name={selectedOption.icon} size="sm" />}
                            <span>{selectedOption.label}</span>
                        </>
                    ) : (
                        <span className="custom-dropdown-placeholder">{placeholder || 'Select...'}</span>
                    )}
                </span>
                <span className={`custom-dropdown-arrow ${isOpen ? 'rotated' : ''}`}>
                    <Icon name="chevronDown" size="sm" />
                </span>
            </button>
            {menu}
        </div>
    );
}

export default CustomDropdown;
