import CustomDropdown from '@gui/components/CustomDropdown/CustomDropdown';
import type { Loose } from '../types';

/** GUI 와 같은 드롭다운. options: [{ value, label, disabled? }], onChange(value) */
export function Dropdown({ className = '', ...props }: Loose) {
    return <CustomDropdown className={`dropdown ${className}`} {...props} />;
}
