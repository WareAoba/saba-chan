import CustomDropdown from '@gui/components/CustomDropdown/CustomDropdown';

/** GUI 와 같은 드롭다운. options: [{ value, label, disabled? }], onChange(value) */
export function Dropdown({ className = '', ...props }) {
    return <CustomDropdown className={`dropdown ${className}`} {...props} />;
}
