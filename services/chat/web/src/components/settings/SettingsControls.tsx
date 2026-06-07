import type { ReactNode, SelectHTMLAttributes } from "react";
import { ChevronDown } from "lucide-react";

export function SelectControl({
  children,
  className,
  ...props
}: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <span className={["select-shell", className ?? ""].join(" ")}>
      <select className="control select-control" {...props}>
        {children}
      </select>
      <ChevronDown
        aria-hidden="true"
        className="select-chevron"
        size={16}
        strokeWidth={2.1}
      />
    </span>
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
    <label className={["grid gap-2", className ?? ""].join(" ")}>
      <span className="control-label">{label}</span>
      {children}
    </label>
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
