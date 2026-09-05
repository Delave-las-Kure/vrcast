/**
 * System notifications about long tasks (FR-084).
 *
 * The decision to notify belongs to the core: only it knows whether the window is out
 * of sight and how long the task actually ran. The wording belongs here, because a
 * notification is read by the same person as the rest of the interface and has to
 * arrive in the same language. Wording it in the core would mean a second set of
 * phrases that could drift from the catalogue — and, worse, phrases frozen in whatever
 * language was chosen when the core was written.
 *
 * A failure here breaks nothing: a notification is a courtesy, not the work. On Linux
 * there may be no notification service at all, and that is not an error to report.
 */

import { useEffect } from "react";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import type { TaskKind, TaskNotifyRequest } from "../../shared/contract";
import { onHiddenToTray, onTaskNotify } from "../../shared/ipc";
import { useLang, useT } from "../../shared/i18n";
import type { Catalogue, Lang } from "../../shared/i18n";
import { renderError } from "../../shared/i18n/render";

function bodyFor(request: TaskNotifyRequest, t: Catalogue, lang: Lang): string {
  if (request.state === "failed") {
    return request.error
      ? renderError(request.error, t, lang).message
      : t.ui.notifications.lookInTasks;
  }
  const done = t.ui.notifications.done;
  const kind = request.kind as TaskKind;
  return kind in done ? done[kind as keyof typeof done] : t.ui.notifications.lookInTasks;
}

export function useTaskNotifications() {
  const t = useT();
  const { lang } = useLang();

  useEffect(() => {
    let stop: (() => void) | undefined;
    let dropped = false;

    onTaskNotify(async (request) => {
      try {
        let allowed = await isPermissionGranted();
        if (!allowed) allowed = (await requestPermission()) === "granted";
        if (!allowed) return;

        sendNotification({
          title:
            request.state === "failed" ? t.ui.notifications.failed : t.ui.notifications.completed,
          body: bodyFor(request, t, lang),
        });
      } catch {
        // No notification service, or permission refused. Nothing to repair.
      }
    })
      .then((unlisten) => {
        if (dropped) unlisten();
        else stop = unlisten;
      })
      .catch(() => {
        // Outside the shell (in tests) there is nothing to listen to.
      });

    return () => {
      dropped = true;
      stop?.();
    };
  }, [t, lang]);
}

/**
 * Where the window went, the first time it goes there (T399, FR-150).
 *
 * ⚠ **This stands in for a check that cannot be made.** Whether the tray icon is visible has
 * no answer: `rect()` on Linux is always `None`, tray events are documented as unsupported
 * there, and nothing reports a failure (R-35). On Windows 11 a new icon goes into the
 * overflow, invisible in a different way. So the window may have vanished into nothing the
 * person can see, and the honest answer is to say where it went.
 *
 * Once, and the core is what knows it is the first time — the fact has to survive a restart,
 * so it lives in its settings, not here. The wording is this side's, as for every other
 * notification.
 */
export function useTrayNotice() {
  const t = useT();

  useEffect(() => {
    let stop: (() => void) | undefined;
    let dropped = false;

    onHiddenToTray(async () => {
      try {
        let allowed = await isPermissionGranted();
        if (!allowed) allowed = (await requestPermission()) === "granted";
        if (!allowed) return;

        sendNotification({
          title: t.ui.notifications.hiddenTitle,
          body: t.ui.notifications.hiddenBody,
        });
      } catch {
        // No notification service, or permission refused. Nothing to repair — and nothing
        // to retry either: the core has already written down that it was said.
      }
    })
      .then((unlisten) => {
        if (dropped) unlisten();
        else stop = unlisten;
      })
      .catch(() => {
        // Outside the shell (in tests) there is nothing to listen to.
      });

    return () => {
      dropped = true;
      stop?.();
    };
  }, [t]);
}
