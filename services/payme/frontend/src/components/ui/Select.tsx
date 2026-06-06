import { SelectHTMLAttributes, forwardRef } from "react";

interface SelectProps extends SelectHTMLAttributes<HTMLSelectElement> {
  label?: string;
  options: { value: string | number; label: string }[];
}

export const Select = forwardRef<HTMLSelectElement, SelectProps>(
  ({ label, options, className = "", ...props }, ref) => {
    return (
      <div className="flex flex-col gap-1">
        {label && (
          <label className="text-xs text-charcoal-600 dark:text-sand-400">
            {label}
          </label>
        )}
        <select
          ref={ref}
          className={`w-full bg-transparent border-b-2 border-sand-300 dark:border-charcoal-600 px-1 py-2.5 md:py-2 text-base md:text-sm text-charcoal-900 dark:text-sand-100 focus:outline-none focus:border-sage-500 dark:focus:border-sage-400 transition-colors cursor-pointer touch-manipulation ${className}`}
          {...props}
        >
          {options.map((opt) => (
            <option key={opt.value} value={opt.value}>
              {opt.label}
            </option>
          ))}
        </select>
      </div>
    );
  }
);

