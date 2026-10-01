/**
 * "Details", folded (T674).
 *
 * The owner's rule: short labels on screens, explanations only where something can go wrong —
 * and even there out of the way until asked for. What a dangerous action will and will not do
 * (the rollback's list, the replaced Caddyfile's fate) is still owed to the person before they
 * agree; it waits here, one click away, instead of standing in a paragraph nobody reads.
 */

import type { ReactNode } from "react";
import { useT } from "../../shared/i18n";

export function More({ children, testId }: { children: ReactNode; testId?: string }) {
  const t = useT();
  return (
    <details className="more" data-testid={testId}>
      <summary>{t.ui.common.more}</summary>
      {children}
    </details>
  );
}
