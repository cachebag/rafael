import { useEffect, useRef, useState } from "react";
import { Check, Copy } from "lucide-react";

export interface CopyButtonProps {
  text: string;
  label: string;
  copiedLabel?: string;
  variant?: "icon" | "labeled";
  className?: string;
}

export function CopyButton({
  text,
  label,
  copiedLabel = "Copied",
  variant = "icon",
  className
}: CopyButtonProps) {
  const [copied, setCopied] = useState(false);
  const timeoutRef = useRef<number | null>(null);
  const buttonLabel = copied ? copiedLabel : label;

  useEffect(() => {
    return () => {
      if (timeoutRef.current !== null) {
        window.clearTimeout(timeoutRef.current);
      }
    };
  }, []);

  async function copyText(): Promise<void> {
    const ok = await writeClipboard(text);
    if (!ok) {
      return;
    }

    setCopied(true);
    if (timeoutRef.current !== null) {
      window.clearTimeout(timeoutRef.current);
    }
    timeoutRef.current = window.setTimeout(() => setCopied(false), 1300);
  }

  return (
    <button
      type="button"
      className={[
        "copy-button",
        variant === "icon" ? "copy-button-icon" : "copy-button-labeled",
        className ?? ""
      ].join(" ")}
      aria-label={buttonLabel}
      title={buttonLabel}
      disabled={text.length === 0}
      onClick={() => void copyText()}
    >
      {copied ? (
        <Check aria-hidden="true" size={14} strokeWidth={2.2} />
      ) : (
        <Copy aria-hidden="true" size={14} strokeWidth={2.2} />
      )}
      <span className={variant === "icon" ? "sr-only" : "copy-button-text"}>
        {buttonLabel}
      </span>
    </button>
  );
}

async function writeClipboard(text: string): Promise<boolean> {
  if (navigator.clipboard !== undefined && window.isSecureContext) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      return writeClipboardFallback(text);
    }
  }

  return writeClipboardFallback(text);
}

function writeClipboardFallback(text: string): boolean {
  const textArea = document.createElement("textarea");
  textArea.value = text;
  textArea.setAttribute("readonly", "");
  textArea.style.position = "fixed";
  textArea.style.left = "-9999px";
  textArea.style.top = "0";
  document.body.appendChild(textArea);
  textArea.select();

  try {
    return document.execCommand("copy");
  } finally {
    document.body.removeChild(textArea);
  }
}
