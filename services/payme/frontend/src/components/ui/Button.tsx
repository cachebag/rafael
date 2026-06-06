import { ButtonHTMLAttributes, ReactNode } from "react";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary" | "danger" | "ghost";
  size?: "sm" | "md" | "lg";
  children: ReactNode;
}

export function Button({
  variant = "primary",
  size = "md",
  children,
  className = "",
  ...props
}: ButtonProps) {
  const baseStyles =
    "inline-flex items-center justify-center font-medium transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed rounded touch-manipulation";

  const variants = {
    primary:
      "bg-sage-600 text-sand-50 hover:bg-sage-700 active:bg-sage-800 dark:bg-sage-500 dark:hover:bg-sage-600 dark:active:bg-sage-700",
    secondary:
      "bg-sand-200 text-charcoal-800 hover:bg-sand-300 active:bg-sand-400 dark:bg-charcoal-800 dark:text-sand-200 dark:hover:bg-charcoal-700 dark:active:bg-charcoal-600",
    danger:
      "bg-terracotta-600 text-sand-50 hover:bg-terracotta-700 active:bg-terracotta-800 dark:active:bg-terracotta-900",
    ghost:
      "bg-transparent hover:bg-sand-200 active:bg-sand-300 dark:hover:bg-charcoal-800 dark:active:bg-charcoal-700 text-charcoal-700 dark:text-sand-300",
  };

  const sizes = {
    sm: "px-4 py-2 md:px-3 md:py-1.5 text-sm md:text-xs min-h-[44px] md:min-h-0",
    md: "px-5 py-2.5 md:px-4 md:py-2 text-base md:text-sm min-h-[44px] md:min-h-0",
    lg: "px-7 py-3.5 md:px-6 md:py-3 text-lg md:text-base min-h-[44px] md:min-h-0",
  };

  return (
    <button
      className={`${baseStyles} ${variants[variant]} ${sizes[size]} ${className}`}
      {...props}
    >
      {children}
    </button>
  );
}

