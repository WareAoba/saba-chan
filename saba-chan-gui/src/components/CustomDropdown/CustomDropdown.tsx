import clsx from 'clsx';
import { Select } from 'radix-ui';
import { type ReactNode, useState } from 'react';
import { Icon } from '../Icon';
import './CustomDropdown.css';

const MENU_GAP = 4;

/**
 * 커스텀 드롭다운 (OS 기본 select 대체)
 * 동작(키보드·포커스·글자로 찾기·화면 밖 회피·Portal)은 Radix Select가 맡고, 모양은 CustomDropdown.css가 맡는다.
 * options: [{ value, label, icon?, mark?, disabled? }]
 * mark: 이름 뒤에 붙는 표시 { icon, label(읽어 주는 이름), color? } (예: 메인 데몬 별)
 */
const Mark = ({ mark }: { mark?: DropdownMark }) =>
    mark ? (
        <span className="custom-dropdown-mark" role="img" aria-label={mark.label} data-tooltip={mark.label}>
            <Icon name={mark.icon} size="sm" color={mark.color} />
        </span>
    ) : null;

/** 이름 뒤에 붙는 표시 */
export interface DropdownMark {
    icon: string;
    /** 읽어 주는 이름 */
    label: string;
    color?: string;
}

export interface DropdownOption {
    value: string | number | boolean;
    label: ReactNode;
    icon?: string;
    mark?: DropdownMark;
    disabled?: boolean;
}

interface CustomDropdownProps {
    value: string | number | boolean | null | undefined;
    // biome-ignore lint/suspicious/noExplicitAny: 호출하는 쪽이 값 종류(문자열·숫자·불리언)에 맞는 핸들러를 넘긴다
    onChange: (value: any) => void;
    options?: DropdownOption[];
    placeholder?: string;
    className?: string;
    disabled?: boolean;
    id?: string;
    ariaLabel?: string;
}

function CustomDropdown({
    value,
    onChange,
    options = [],
    placeholder,
    className,
    disabled,
    id,
    ariaLabel,
}: CustomDropdownProps) {
    const [isOpen, setIsOpen] = useState(false);
    // Radix 값은 빈 칸이 아닌 문자열이어야 하므로 항목 순번을 열쇠로 쓰고, 고르면 원래 값으로 돌려준다
    const key = (i: number) => `o${i}`;
    const selectedIndex = options.findIndex((o) => String(o.value) === String(value));
    const selectedOption = options[selectedIndex];

    const handleChange = (k: string) => {
        const option = options[Number(k.slice(1))];
        if (option && String(option.value) !== String(value)) onChange(option.value);
    };

    return (
        <Select.Root
            value={selectedIndex >= 0 ? key(selectedIndex) : ''}
            onValueChange={handleChange}
            open={isOpen}
            onOpenChange={setIsOpen}
            disabled={disabled}
        >
            <div className={clsx('custom-dropdown', className, { open: isOpen, disabled })}>
                <Select.Trigger id={id} className="custom-dropdown-trigger" aria-label={ariaLabel}>
                    <span className="custom-dropdown-value">
                        {selectedOption ? (
                            <>
                                {selectedOption.icon && <Icon name={selectedOption.icon} size="sm" />}
                                {/* 보이지 않는 모든 이름을 같은 칸에 겹쳐, 폭을 내용에 맞출 때 가장 긴 이름에 맞춘다 (고를 때 폭이 튀지 않게) */}
                                <span className="custom-dropdown-sizer">
                                    <span className="custom-dropdown-label" data-find="value">
                                        {selectedOption.label}
                                    </span>
                                    {/* 글자는 CSS attr()로만 그려 DOM 글자·읽기에 섞이지 않는다 */}
                                    {options.map((o) =>
                                        typeof o.label === 'string' ? (
                                            <span
                                                key={String(o.value)}
                                                className="custom-dropdown-ghost"
                                                data-label={o.label}
                                            />
                                        ) : null,
                                    )}
                                </span>
                                <Mark mark={selectedOption.mark} />
                            </>
                        ) : (
                            <span className="custom-dropdown-placeholder">{placeholder || 'Select...'}</span>
                        )}
                    </span>
                    <Select.Icon className={clsx('custom-dropdown-arrow', { rotated: isOpen })}>
                        <Icon name="chevronDown" size="sm" />
                    </Select.Icon>
                </Select.Trigger>
            </div>
            <Select.Portal>
                <Select.Content
                    className="custom-dropdown-menu"
                    position="popper"
                    side="bottom"
                    align="start"
                    sideOffset={MENU_GAP}
                    collisionPadding={MENU_GAP * 2}
                >
                    <Select.Viewport className="custom-dropdown-scroll">
                        {options.map((option, i) => (
                            <Select.Item
                                key={String(option.value)}
                                value={key(i)}
                                disabled={option.disabled}
                                textValue={typeof option.label === 'string' ? option.label : undefined}
                                className="custom-dropdown-item"
                            >
                                <span className="custom-dropdown-item-content">
                                    {option.icon && <Icon name={option.icon} size="sm" />}
                                    <Select.ItemText>{option.label}</Select.ItemText>
                                    <Mark mark={option.mark} />
                                </span>
                                <Select.ItemIndicator className="custom-dropdown-check">
                                    <Icon name="check" size="sm" />
                                </Select.ItemIndicator>
                            </Select.Item>
                        ))}
                    </Select.Viewport>
                </Select.Content>
            </Select.Portal>
        </Select.Root>
    );
}

export default CustomDropdown;
