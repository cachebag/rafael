import { InputHTMLAttributes, forwardRef } from "react";

interface InputProps extends InputHTMLAttributes<HTMLInputElement> {
  label?: string;
}

export const Input = forwardRef<HTMLInputElement, InputProps>(
  ({ label, className = "", ...props }, ref) => {
    return (
      <div className="flex flex-col gap-1">
        {label && (
          <label className="text-xs text-charcoal-500 dark:text-charcoal-400">
            {label}
          </label>
        )}
        <input
          ref={ref}
          className={`w-full rounded-md border border-sand-300 bg-charcoal-50 px-3 py-2.5 text-base text-charcoal-900 outline-none transition-colors placeholder:text-charcoal-400 focus:border-sage-500 md:py-2 md:text-sm dark:border-charcoal-700 dark:bg-charcoal-950/60 dark:text-sand-100 dark:placeholder:text-charcoal-500 dark:focus:border-sage-500 touch-manipulation ${className}`}
          {...props}
        />
      </div>
    );
  }
);
