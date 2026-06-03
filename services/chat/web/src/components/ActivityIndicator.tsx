interface ActivityIndicatorProps {
  label?: string;
  compact?: boolean;
}

export function ActivityIndicator({
  label = "Working",
  compact = false
}: ActivityIndicatorProps) {
  return (
    <div
      className={[
        "activity-indicator",
        compact ? "activity-indicator-compact" : ""
      ].join(" ")}
      role="status"
      aria-live="polite"
      aria-label={label}
    >
      <span className="activity-dots" aria-hidden="true">
        <span />
        <span />
        <span />
      </span>
    </div>
  );
}
