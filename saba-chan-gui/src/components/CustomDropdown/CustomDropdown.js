import clsx from 'clsx';
import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import ReactDOM from 'react-dom';
import { Icon } from '../Icon';
import './CustomDropdown.css';

const MENU_GAP = 4;
const MENU_MAX = 280;
const CLOSE_MS = 120;

/**
 * 커스텀 드롭다운 (OS 기본 select 대체)
 * 메뉴는 Portal 팝업으로 띄운다. 아래 자리가 모자라면 위로 연다.
 * options: [{ value, label, icon?, mark?, disabled? }]
 * mark: 이름 뒤에 붙는 표시 { icon, label(읽어 주는 이름), color? } (예: 메인 데몬 별)
 */
const Mark = ({ mark }) =>
    mark ? (
        <span className="custom-dropdown-mark" role="img" aria-label={mark.label} data-tooltip={mark.label}>
            <Icon name={mark.icon} size="sm" color={mark.color} />
        </span>
    ) : null;

function CustomDropdown({ value, onChange, options = [], placeholder, className, disabled, id, ariaLabel }) {
    const [isOpen, setIsOpen] = useState(false);
    const [isClosing, setIsClosing] = useState(false);
    const [menuStyle, setMenuStyle] = useState({});
    const [placement, setPlacement] = useState('bottom');
    const [active, setActive] = useState(-1);
    const triggerRef = useRef(null);
    const menuRef = useRef(null);
    const closeTimer = useRef(null);
    const autoId = useId();
    const triggerId = id || `dropdown-${autoId}`;
    const listId = `${triggerId}-list`;

    const selectedIndex = options.findIndex((o) => String(o.value) === String(value));
    const selectedOption = options[selectedIndex];

    // 트리거 위치로 메뉴 좌표를 정한다 (아래가 좁으면 위로)
    const updateMenuPosition = useCallback(() => {
        if (!triggerRef.current) return;
        const rect = triggerRef.current.getBoundingClientRect();
        const below = window.innerHeight - rect.bottom - MENU_GAP;
        const above = rect.top - MENU_GAP;
        const up = below < Math.min(MENU_MAX, options.length * 36 + 8) && above > below;
        setPlacement(up ? 'top' : 'bottom');
        // 화면 오른쪽 절반에 있으면 메뉴를 오른쪽 끝에 맞춘다 (긴 항목이 좁은 화면 밖으로 나가지 않게)
        const alignRight = rect.left + rect.width / 2 > window.innerWidth / 2;
        setMenuStyle({
            position: 'fixed',
            ...(alignRight ? { right: window.innerWidth - rect.right } : { left: rect.left }),
            maxWidth: (alignRight ? rect.right : window.innerWidth - rect.left) - MENU_GAP * 2,
            minWidth: rect.width,
            zIndex: 10000,
            ...(up ? { bottom: window.innerHeight - rect.top + MENU_GAP } : { top: rect.bottom + MENU_GAP }),
            '--dropdown-max': `${Math.max(120, Math.min(MENU_MAX, up ? above : below) - 8)}px`,
        });
    }, [options.length]);

    const open = useCallback(() => {
        if (disabled) return;
        clearTimeout(closeTimer.current);
        setIsClosing(false);
        setActive(selectedIndex >= 0 ? selectedIndex : options.findIndex((o) => !o.disabled));
        setIsOpen(true);
    }, [disabled, selectedIndex, options]);

    const close = useCallback(() => {
        setIsClosing(true);
        clearTimeout(closeTimer.current);
        closeTimer.current = setTimeout(() => {
            setIsOpen(false);
            setIsClosing(false);
        }, CLOSE_MS);
    }, []);

    useEffect(() => () => clearTimeout(closeTimer.current), []);

    // 열릴 때 위치 계산 + 스크롤/리사이즈 추적
    useLayoutEffect(() => {
        if (!isOpen) return;
        updateMenuPosition();
        window.addEventListener('scroll', updateMenuPosition, true);
        window.addEventListener('resize', updateMenuPosition);
        return () => {
            window.removeEventListener('scroll', updateMenuPosition, true);
            window.removeEventListener('resize', updateMenuPosition);
        };
    }, [isOpen, updateMenuPosition]);

    // 고른 항목이 보이게 스크롤
    useEffect(() => {
        if (!isOpen || active < 0) return;
        menuRef.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView?.({ block: 'nearest' });
    }, [isOpen, active]);

    // 바깥을 누르면 닫는다 (트리거 + 포탈 메뉴 둘 다 확인)
    useEffect(() => {
        if (!isOpen || isClosing) return;
        const handleClickOutside = (e) => {
            if (triggerRef.current?.contains(e.target) || menuRef.current?.contains(e.target)) return;
            close();
        };
        document.addEventListener('mousedown', handleClickOutside);
        return () => document.removeEventListener('mousedown', handleClickOutside);
    }, [isOpen, isClosing, close]);

    const handleSelect = (option) => {
        if (option.disabled) return;
        if (String(option.value) !== String(value)) onChange(option.value);
        close();
        triggerRef.current?.focus();
    };

    const move = (dir) => {
        if (!options.length) return;
        let i = active;
        for (let n = 0; n < options.length; n++) {
            i = (i + dir + options.length) % options.length;
            if (!options[i].disabled) break;
        }
        setActive(i);
    };

    const shown = isOpen && !isClosing;

    const handleKeyDown = (e) => {
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
            e.preventDefault();
            if (!shown) open();
            else move(e.key === 'ArrowDown' ? 1 : -1);
        } else if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            if (!shown) open();
            else if (options[active]) handleSelect(options[active]);
        } else if (e.key === 'Escape' && shown) {
            e.preventDefault();
            e.stopPropagation();
            close();
        } else if (e.key === 'Tab' && shown) {
            close();
        }
    };

    // Portal로 body에 그리는 팝업
    const menu = isOpen
        ? ReactDOM.createPortal(
              <div
                  className={clsx('custom-dropdown-menu', `placement-${placement}`, { closing: isClosing })}
                  ref={menuRef}
                  style={menuStyle}
              >
                  <div className="custom-dropdown-scroll" role="listbox" id={listId} aria-labelledby={triggerId}>
                      {options.map((option, i) => {
                          const isSelected = i === selectedIndex;
                          return (
                              // biome-ignore lint/a11y/useKeyWithClickEvents: 키보드는 트리거(combobox)가 받는다
                              <div
                                  key={String(option.value)}
                                  data-index={i}
                                  role="option"
                                  tabIndex={-1}
                                  aria-selected={isSelected}
                                  aria-disabled={option.disabled || undefined}
                                  className={clsx('custom-dropdown-item', {
                                      selected: isSelected,
                                      active: i === active,
                                      disabled: option.disabled,
                                  })}
                                  onMouseEnter={() => !option.disabled && setActive(i)}
                                  onClick={() => handleSelect(option)}
                              >
                                  <span className="custom-dropdown-item-content">
                                      {option.icon && <Icon name={option.icon} size="sm" />}
                                      <span>{option.label}</span>
                                      <Mark mark={option.mark} />
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
              document.body,
          )
        : null;

    return (
        <div className={clsx('custom-dropdown', className, { open: shown, disabled })}>
            <button
                id={triggerId}
                className="custom-dropdown-trigger"
                onClick={() => (shown ? close() : open())}
                onKeyDown={handleKeyDown}
                ref={triggerRef}
                type="button"
                role="combobox"
                aria-label={ariaLabel}
                aria-haspopup="listbox"
                aria-expanded={shown}
                aria-controls={listId}
                disabled={disabled}
            >
                <span className="custom-dropdown-value">
                    {selectedOption ? (
                        <>
                            {selectedOption.icon && <Icon name={selectedOption.icon} size="sm" />}
                            <span className="custom-dropdown-label">{selectedOption.label}</span>
                            <Mark mark={selectedOption.mark} />
                        </>
                    ) : (
                        <span className="custom-dropdown-placeholder">{placeholder || 'Select...'}</span>
                    )}
                </span>
                <span className={clsx('custom-dropdown-arrow', { rotated: shown })}>
                    <Icon name="chevronDown" size="sm" />
                </span>
            </button>
            {menu}
        </div>
    );
}

export default CustomDropdown;
