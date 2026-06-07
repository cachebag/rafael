import { Trash2 } from "lucide-react";

interface DataSettingsProps {
  conversationCount: number;
  controlsDisabled: boolean;
  purgeConfirm: string;
  onPurgeConfirmChange: (value: string) => void;
  onPurgeConversations: () => void;
}

export function DataSettings({
  conversationCount,
  controlsDisabled,
  purgeConfirm,
  onPurgeConfirmChange,
  onPurgeConversations
}: DataSettingsProps) {
  const purgeReady = purgeConfirm.trim() === "PURGE";

  return (
    <section className="settings-section settings-danger-section">
      <h3 className="settings-section-title">Danger zone</h3>
      <div className="settings-danger-layout">
        <div className="min-w-0">
          <p className="settings-danger-title">Purge all chats</p>
          <p className="settings-danger-copy">
            {conversationCountLabel(conversationCount)}. Type PURGE to confirm.
          </p>
        </div>
        <div className="settings-danger-controls">
          <input
            className="control settings-danger-input"
            value={purgeConfirm}
            placeholder="PURGE"
            disabled={controlsDisabled || conversationCount === 0}
            spellCheck={false}
            onChange={(event) => onPurgeConfirmChange(event.target.value)}
          />
          <button
            type="button"
            className="button-danger"
            disabled={controlsDisabled || conversationCount === 0 || !purgeReady}
            onClick={onPurgeConversations}
          >
            <Trash2 aria-hidden="true" size={15} strokeWidth={2.1} />
            Purge chats
          </button>
        </div>
      </div>
    </section>
  );
}

function conversationCountLabel(count: number): string {
  return count === 1 ? "1 saved conversation" : `${count} saved conversations`;
}
