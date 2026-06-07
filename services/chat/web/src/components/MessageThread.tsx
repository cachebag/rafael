import { memo } from "react";
import { Database, Globe2 } from "lucide-react";
import type {
  ChatMemoryUse,
  ChatMessageMetadata,
  ChatMessageRecord,
  ChatSource,
  ToolActivity
} from "../types";
import { ActivityIndicator, ToolActivityIndicator } from "./ActivityIndicator";
import { LazyCopyButton } from "./LazyCopyButton";
import { MarkdownContent } from "./MarkdownContent";

interface MessageThreadProps {
  messages: ChatMessageRecord[];
  busy: boolean;
  toolActivity: ToolActivity | null;
}

export const MessageThread = memo(function MessageThread({
  messages,
  busy,
  toolActivity
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
            {toolActivity === null ? (
              <ActivityIndicator label="Waiting for response" />
            ) : (
              <ToolActivityIndicator label={toolActivityLabel(toolActivity)} />
            )}
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

function toolActivityLabel(activity: ToolActivity): string {
  if (activity.name === "web_search") {
    return "searching the web...";
  }
  if (activity.name === "fetch_url") {
    return "reading source...";
  }
  return "using tool...";
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
            <MessageMetadataFooter metadata={message.metadata} />
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

interface MessageMetadataFooterProps {
  metadata?: ChatMessageMetadata;
}

function MessageMetadataFooter({ metadata }: MessageMetadataFooterProps) {
  const memories = uniqueMemories(metadata?.memories ?? []);
  const hasWeb = hasWebToolUse(metadata);
  if (!hasWeb && memories.length === 0) {
    return null;
  }

  const sources = uniqueSources(metadata?.sources ?? []).slice(0, 3);
  const sourceCount = uniqueSources(metadata?.sources ?? []).length;

  return (
    <footer className="grid gap-2">
      {hasWeb ? (
        <div className="message-source-footer" aria-label="Web source note">
          <Globe2 aria-hidden="true" size={13} strokeWidth={1.8} />
          <span>searched web</span>
          {sourceCount > 0 ? <span>{sourceCount} sources</span> : null}
          <span>sources may be incomplete</span>
          {sources.length > 0 ? (
            <span className="message-source-links">
              {sources.map((source) => (
                <a
                  key={source.url}
                  href={source.url}
                  target="_blank"
                  rel="noreferrer"
                  title={source.title ?? source.url}
                >
                  {sourceLabel(source)}
                </a>
              ))}
            </span>
          ) : null}
        </div>
      ) : null}
      {memories.length > 0 ? (
        <details className="message-memory-footer">
          <summary>
            <Database aria-hidden="true" size={13} strokeWidth={1.8} />
            <span>{memories.length === 1 ? "1 memory" : `${memories.length} memories`}</span>
          </summary>
          <div className="message-memory-list">
            {memories.map((memory) => (
              <p key={memory.id}>
                <span>{memory.kind}</span>
                {memory.content}
              </p>
            ))}
          </div>
        </details>
      ) : null}
    </footer>
  );
}

function hasWebToolUse(metadata?: ChatMessageMetadata): boolean {
  return (
    metadata?.toolUses?.some((toolUse) =>
      toolUse.name === "web_search" || toolUse.name === "fetch_url"
    ) ?? false
  );
}

function uniqueSources(sources: ChatSource[]): ChatSource[] {
  const seen = new Set<string>();
  const unique: ChatSource[] = [];

  for (const source of sources) {
    const url = source.url.trim();
    if (url === "" || seen.has(url)) {
      continue;
    }
    seen.add(url);
    unique.push({ ...source, url });
  }

  return unique;
}

function uniqueMemories(memories: ChatMemoryUse[]): ChatMemoryUse[] {
  const seen = new Set<string>();
  const unique: ChatMemoryUse[] = [];

  for (const memory of memories) {
    if (seen.has(memory.id)) {
      continue;
    }
    seen.add(memory.id);
    unique.push(memory);
  }

  return unique;
}

function sourceLabel(source: ChatSource): string {
  const title = source.title?.trim();
  if (title !== undefined && title !== "") {
    return truncateText(title, 34);
  }

  try {
    return new URL(source.url).hostname.replace(/^www\./, "");
  } catch {
    return truncateText(source.url, 34);
  }
}

function truncateText(value: string, maxLength: number): string {
  return value.length > maxLength ? `${value.slice(0, maxLength - 1)}...` : value;
}

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
