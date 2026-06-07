import { LogOut } from "lucide-react";
import type { AuthUser } from "../../types";

interface AccountSettingsProps {
  user: AuthUser;
  controlsDisabled: boolean;
  onLogout: () => void;
}

export function AccountSettings({
  user,
  controlsDisabled,
  onLogout
}: AccountSettingsProps) {
  return (
    <section className="settings-section">
      <h3 className="settings-section-title">Account</h3>
      <div className="settings-account-row">
        <div className="min-w-0">
          <p className="settings-account-name">{user.firstName}</p>
          <p className="settings-account-copy">@{user.username} · signed in on this browser.</p>
        </div>
        <button
          type="button"
          className="button-secondary settings-logout-button"
          disabled={controlsDisabled}
          onClick={onLogout}
        >
          <LogOut aria-hidden="true" size={15} strokeWidth={2.1} />
          Sign out
        </button>
      </div>
    </section>
  );
}
