import { useState } from "react";
import type { FormEvent } from "react";

type AuthMode = "login" | "register";

interface AuthPanelProps {
  busy: boolean;
  error: string | null;
  onLogin: (username: string, password: string) => Promise<void>;
  onRegister: (username: string, password: string) => Promise<void>;
}

export function AuthPanel({ busy, error, onLogin, onRegister }: AuthPanelProps) {
  const [mode, setMode] = useState<AuthMode>("login");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [localError, setLocalError] = useState<string | null>(null);
  const canSubmit = username.trim() !== "" && password !== "" && !busy;

  async function submit(event: FormEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault();
    if (!canSubmit) {
      return;
    }

    setLocalError(null);
    try {
      if (mode === "login") {
        await onLogin(username, password);
      } else {
        await onRegister(username, password);
      }
    } catch (cause) {
      setLocalError(cause instanceof Error ? cause.message : "authentication failed");
    }
  }

  return (
    <main className="auth-shell min-h-dvh text-[var(--text)]">
      <section className="auth-panel" aria-labelledby="auth-title">
        <div className="auth-brand">
          <p className="auth-kicker">rafael</p>
          <h1 id="auth-title">{mode === "login" ? "Sign in" : "Register"}</h1>
        </div>

        <div className="auth-mode-switch" role="tablist" aria-label="Authentication mode">
          <button
            type="button"
            role="tab"
            aria-selected={mode === "login"}
            className={mode === "login" ? "auth-mode-active" : ""}
            disabled={busy}
            onClick={() => setMode("login")}
          >
            Sign in
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={mode === "register"}
            className={mode === "register" ? "auth-mode-active" : ""}
            disabled={busy}
            onClick={() => setMode("register")}
          >
            Register
          </button>
        </div>

        <form className="auth-form" onSubmit={(event) => void submit(event)}>
          <label className="grid gap-2">
            <span className="control-label">Username</span>
            <input
              className="control"
              value={username}
              autoComplete="username"
              placeholder="Akrm"
              disabled={busy}
              onChange={(event) => setUsername(event.target.value)}
            />
          </label>

          <label className="grid gap-2">
            <span className="control-label">Password</span>
            <input
              className="control"
              type="password"
              value={password}
              autoComplete={mode === "login" ? "current-password" : "new-password"}
              disabled={busy}
              onChange={(event) => setPassword(event.target.value)}
            />
          </label>

          {mode === "register" ? (
            <p className="auth-note">Allowed first names: Akrm, Nowar, Sofia.</p>
          ) : null}

          {localError !== null || error !== null ? (
            <p className="auth-error">{localError ?? error}</p>
          ) : null}

          <button type="submit" className="button-primary auth-submit" disabled={!canSubmit}>
            {mode === "login" ? "Sign in" : "Create account"}
          </button>
        </form>
      </section>
    </main>
  );
}
