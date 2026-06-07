import { useEffect, useId, useRef, useState } from "react";
import { Check, ChevronDown } from "lucide-react";

export interface CustomSelectOption {
  value: string;
  label: string;
  detail?: string;
  disabled?: boolean;
}

interface CustomSelectProps {
  value: string;
  options: CustomSelectOption[];
  disabled?: boolean;
  ariaLabel: string;
  className?: string;
  menuClassName?: string;
  showSelectedDetail?: boolean;
  onChange: (value: string) => void;
}

export function CustomSelect({
  value,
  options,
  disabled = false,
  ariaLabel,
  className,
  menuClassName,
  showSelectedDetail = false,
  onChange
}: CustomSelectProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const buttonId = useId();
  const listboxId = useId();
  const selectedOption =
    options.find((option) => option.value === value) ?? options[0] ?? null;

  useEffect(() => {
    if (!open) {
      return;
    }

    function handlePointerDown(event: PointerEvent): void {
      const root = rootRef.current;
      if (root !== null && !root.contains(event.target as Node)) {
        setOpen(false);
      }
    }

    function handleKeyDown(event: KeyboardEvent): void {
      if (event.key === "Escape") {
        setOpen(false);
      }
    }

    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [open]);

  function choose(option: CustomSelectOption): void {
    if (option.disabled) {
      return;
    }
    onChange(option.value);
    setOpen(false);
  }

  return (
    <div ref={rootRef} className={["custom-select", className ?? ""].join(" ")}>
      <button
        id={buttonId}
        type="button"
        className="custom-select-button"
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={listboxId}
        disabled={disabled || options.length === 0}
        onClick={() => setOpen((current) => !current)}
      >
        <span className="custom-select-value">
          <span className="custom-select-label">
            {selectedOption?.label ?? "No options"}
          </span>
          {showSelectedDetail && selectedOption?.detail !== undefined ? (
            <span className="custom-select-detail">{selectedOption.detail}</span>
          ) : null}
        </span>
        <ChevronDown
          aria-hidden="true"
          className="custom-select-chevron"
          size={16}
          strokeWidth={2.1}
        />
      </button>

      {open ? (
        <div
          id={listboxId}
          className={["custom-select-menu", menuClassName ?? ""].join(" ")}
          role="listbox"
          aria-labelledby={buttonId}
        >
          {options.map((option) => {
            const selected = option.value === value;
            return (
              <button
                key={option.value}
                type="button"
                role="option"
                aria-selected={selected}
                className={[
                  "custom-select-option",
                  selected ? "custom-select-option-selected" : ""
                ].join(" ")}
                disabled={option.disabled}
                onClick={() => choose(option)}
              >
                <span className="custom-select-option-copy">
                  <span className="custom-select-option-label">{option.label}</span>
                  {option.detail !== undefined ? (
                    <span className="custom-select-option-detail">{option.detail}</span>
                  ) : null}
                </span>
                {selected ? (
                  <Check
                    aria-hidden="true"
                    className="custom-select-option-check"
                    size={15}
                    strokeWidth={2.1}
                  />
                ) : null}
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
