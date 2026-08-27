/**
 * T056 — copying a viewer link (FR-016).
 *
 * When a CDN is configured there are two links, and the choice is left to the person:
 * they cost different things. A link through the server itself is not blocked in
 * Russia; through a CDN it is faster, but it depends on an intermediary and will serve
 * a replaced file from cache for a while.
 *
 * Confirming the copy is not optional, and it stays for a couple of seconds: without
 * it a person does not know whether the click worked and clicks again.
 */

import { useEffect, useRef, useState } from "react";
import type { FileView } from "../../shared/contract";
import { useT } from "../../shared/i18n";

/** Which of the two links was copied — or that copying failed. */
type Copied = "server" | "cdn" | "failed";

export function CopyLink({ file }: { file: FileView }) {
  const [copied, setCopied] = useState<Copied | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const t = useT();

  // The copied mark is cleared by a timer, and the timer must be stopped on unmount:
  // otherwise it fires on a node already taken off the page.
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );

  const copy = async (url: string, what: Copied) => {
    try {
      await navigator.clipboard.writeText(url);
      setCopied(what);
    } catch {
      // The clipboard may be unavailable. Staying quiet is not an option: a person
      // would take it as copied and paste the wrong thing.
      setCopied("failed");
    }
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => setCopied(null), 2000);
  };

  if (!file.exists_on_server) {
    return (
      <span className="copy-link__dead" title={t.ui.library.linkDeadTitle}>
        {t.ui.library.linkDead}
      </span>
    );
  }

  const said =
    copied === "failed"
      ? t.ui.library.linkCopyFailed
      : copied === "cdn"
        ? t.ui.library.linkCopiedCdn
        : t.ui.library.linkCopiedServer;

  return (
    <span className="copy-link">
      <button onClick={() => void copy(file.origin_url, "server")}>
        {file.cdn_url ? t.ui.library.linkFromServer : t.ui.library.linkCopy}
      </button>
      {file.cdn_url && (
        <button onClick={() => void copy(file.cdn_url!, "cdn")}>{t.ui.library.linkViaCdn}</button>
      )}
      {copied && (
        <span className="copy-link__done" role="status">
          {said}
        </span>
      )}
    </span>
  );
}
