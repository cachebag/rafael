interface ProgressBarProps {
  value: number;
  max: number;
  showOverage?: boolean;
  invertColors?: boolean;
}

export function ProgressBar({ value, max, showOverage = true, invertColors = false }: ProgressBarProps) {
  const percentage = max > 0 ? (value / max) * 100 : 0;
  const isOver = value > max;
  const overage = value - max;

  const getColor = () => {
    if (invertColors) {
      // Green for high progress (savings goals)
      if (percentage >= 100) return "bg-sage-500";
      if (percentage >= 80) return "bg-sage-500";
      if (percentage >= 50) return "bg-sand-500";
      return "bg-terracotta-500";
    } else {
      // Red for high usage (spending)
      if (percentage >= 100) return "bg-terracotta-500";
      if (percentage >= 80) return "bg-terracotta-400";
      if (percentage >= 50) return "bg-sand-500";
      return "bg-sage-500";
    }
  };

  return (
    <div className="space-y-1">
      <div className="h-2 bg-sand-200 dark:bg-charcoal-800 overflow-hidden">
        <div
          className={`h-full transition-all duration-300 ${getColor()}`}
          style={{ width: `${Math.min(percentage, 100)}%` }}
        />
      </div>
      {isOver && showOverage && (
        <div className="text-xs text-terracotta-600 dark:text-terracotta-400">
          +${overage.toFixed(2)} over
        </div>
      )}
    </div>
  );
}

