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
      <span className="activity-glyph" aria-hidden="true">
        <span className="activity-thread activity-thread-primary" />
        <span className="activity-thread activity-thread-secondary" />
        <span className="activity-node activity-node-a" />
        <span className="activity-node activity-node-b" />
        <span className="activity-node activity-node-c" />
        <span />
      </span>
    </div>
  );
}
