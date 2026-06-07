import type { ReactNode } from "react";
import { CustomSelect, type CustomSelectOption } from "../CustomSelect";

export function SelectControl({
  options,
  value,
  disabled,
  ariaLabel,
  className,
  onChange
}: {
  options: CustomSelectOption[];
  value: string;
  disabled?: boolean;
  ariaLabel: string;
  className?: string;
  onChange: (value: string) => void;
}) {
  return (
    <CustomSelect
      className={className}
      value={value}
      options={options}
      disabled={disabled}
      ariaLabel={ariaLabel}
      onChange={onChange}
    />
  );
}

export function ToggleField({
  label,
  checked,
  disabled,
  onChange
}: {
  label: string;
  checked: boolean;
  disabled: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="settings-toggle-row">
      <input
        className="settings-toggle-input"
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span className="settings-toggle-copy">{label}</span>
      <span className="settings-toggle-switch" aria-hidden="true">
        <span className="settings-toggle-thumb" />
      </span>
    </label>
  );
}

export function Field({
  label,
  children,
  className
}: {
  label: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={["grid gap-2", className ?? ""].join(" ")}>
      <span className="control-label">{label}</span>
      {children}
    </div>
  );
}

export function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div className="settings-detail">
      <span className="control-label">{label}</span>
      <span className="settings-detail-value" title={value}>
        {value}
      </span>
    </div>
  );
}
