import type { ReactNode } from "react";
import { CircleHelp } from "lucide-react";
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
  description,
  checked,
  disabled,
  onChange
}: {
  label: string;
  description?: string;
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
      <span className="settings-toggle-copy">
        {label}
        {description !== undefined ? <InfoTip text={description} /> : null}
      </span>
      <span className="settings-toggle-switch" aria-hidden="true">
        <span className="settings-toggle-thumb" />
      </span>
    </label>
  );
}

export function Field({
  label,
  description,
  children,
  className
}: {
  label: string;
  description?: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={["grid gap-2", className ?? ""].join(" ")}>
      <span className="control-label">
        {label}
        {description !== undefined ? <InfoTip text={description} /> : null}
      </span>
      {children}
    </div>
  );
}

export function InfoTip({ text }: { text: string }) {
  return (
    <span className="info-tip" tabIndex={0} aria-label={text} data-tooltip={text}>
      <CircleHelp aria-hidden="true" size={13} strokeWidth={2.1} />
    </span>
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
