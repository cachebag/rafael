import { isValidElement, memo, useMemo, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import type { Components } from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import remarkGfm from "remark-gfm";
import { LazyCopyButton } from "./LazyCopyButton";

interface MarkdownContentProps {
  content: string;
  copyEnabled: boolean;
}

export const MarkdownContent = memo(function MarkdownContent({
  content,
  copyEnabled
}: MarkdownContentProps) {
  const components = useMemo<Components>(
    () => ({
      pre({ children, ...props }) {
        const code = getTextContent(children).replace(/\n$/, "");

        return (
          <div className="code-block-shell">
            {copyEnabled ? (
              <LazyCopyButton
                text={code}
                label="Copy code"
                variant="icon"
                className="code-copy-button"
              />
            ) : null}
            <pre {...props}>{children}</pre>
          </div>
        );
      }
    }),
    [copyEnabled]
  );

  return (
    <div className="markdown-content">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[[rehypeHighlight, { detect: true }]]}
        components={components}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
});

function getTextContent(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") {
    return String(node);
  }

  if (Array.isArray(node)) {
    return node.map(getTextContent).join("");
  }

  if (isValidElement<{ children?: ReactNode }>(node)) {
    return getTextContent(node.props.children);
  }

  return "";
}
