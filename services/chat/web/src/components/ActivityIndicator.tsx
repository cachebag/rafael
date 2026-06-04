import { Globe2 } from "lucide-react";

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

interface ToolActivityIndicatorProps {
  label: string;
}

export function ToolActivityIndicator({ label }: ToolActivityIndicatorProps) {
  return (
    <div className="tool-activity-indicator" role="status" aria-live="polite">
      <Globe2 aria-hidden="true" size={15} strokeWidth={1.9} />
      <span>{label}</span>
    </div>
  );
}
