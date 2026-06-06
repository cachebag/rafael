import { ReactNode } from "react";

interface CardProps {
  children: ReactNode;
  className?: string;
}

export function Card({ children, className = "" }: CardProps) {
  return (
    <div
      className={`bg-white dark:bg-charcoal-900 border border-sand-300 dark:border-charcoal-800 p-6 shadow-sm ${className}`}
    >
      {children}
    </div>
  );
}

