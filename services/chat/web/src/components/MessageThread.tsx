import { memo } from "react";
import type { ChatMessageRecord } from "../types";
import { ActivityIndicator } from "./ActivityIndicator";
import { LazyCopyButton } from "./LazyCopyButton";
import { MarkdownContent } from "./MarkdownContent";

interface MessageThreadProps {
  messages: ChatMessageRecord[];
  busy: boolean;
}

export const MessageThread = memo(function MessageThread({
  messages,
  busy
}: MessageThreadProps) {
  const showPendingResponse =
    busy && messages.length > 0 && messages.at(-1)?.role === "user";

  return (
    <div className="grid w-full min-w-0 gap-5">
      {messages.map((message, index) => (
        <MessageBubble
          key={message.id}
          message={message}
          copyEnabled={canCopyMessage(message, index, messages.length, busy)}
        />
      ))}
      {showPendingResponse ? (
        <article className="flex w-full min-w-0 justify-start">
          <div className="message-activity">
            <ActivityIndicator label="Waiting for response" />
          </div>
        </article>
      ) : null}
    </div>
  );
});

interface MessageBubbleProps {
  message: ChatMessageRecord;
  copyEnabled: boolean;
}

const MessageBubble = memo(function MessageBubble({
  message,
  copyEnabled
}: MessageBubbleProps) {
  return (
    <article
      className={[
        "flex w-full min-w-0",
        message.role === "user" ? "justify-end" : "justify-start"
      ].join(" ")}
    >
      <div
        className={[
          "message-bubble rounded-md border px-4 py-3 text-sm leading-6",
          message.role === "user"
            ? "message-bubble-user whitespace-pre-wrap border-[var(--line)] bg-[var(--panel)] shadow-[var(--shadow-soft)]"
            : "message-bubble-model border-transparent bg-[var(--assistant-bg)]",
          copyEnabled ? "message-bubble-copyable" : ""
        ].join(" ")}
      >
        {message.role === "user" ? (
          message.content
        ) : (
          <div className="message-output">
            <MarkdownContent content={message.content} copyEnabled={copyEnabled} />
            {copyEnabled ? (
              <div className="message-output-actions">
                <LazyCopyButton
                  text={message.content}
                  label="Copy response"
                  variant="icon"
                  className="message-copy-button"
                />
              </div>
            ) : null}
          </div>
        )}
      </div>
    </article>
  );
});

function canCopyMessage(
  message: ChatMessageRecord,
  index: number,
  messageCount: number,
  busy: boolean
): boolean {
  return (
    message.role !== "user" &&
    message.content.trim().length > 0 &&
    !(busy && index === messageCount - 1)
  );
}
