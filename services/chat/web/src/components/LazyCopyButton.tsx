import { lazy, Suspense } from "react";
import type { CopyButtonProps } from "./CopyButton";

const DeferredCopyButton = lazy(async () => {
  const module = await import("./CopyButton");
  return { default: module.CopyButton };
});

export function LazyCopyButton(props: CopyButtonProps) {
  return (
    <Suspense fallback={null}>
      <DeferredCopyButton {...props} />
    </Suspense>
  );
}
