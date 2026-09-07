/**
 * The deployment steps as a list — before the agreement and while it runs (FR-122, FR-123).
 *
 * One list for both, deliberately. Somebody who has just read what will be done looks at the
 * same thing while it is being done: the same steps, in the same order, with the marks
 * changing. Two different lists would make them hunt for where they had got to.
 *
 * **"Cannot be done here" is not "done".** It is an answer from the check, not the absence of
 * one: in a container neither swap nor the kernel settings can be touched, and a run that
 * folded that into "ready" would report a fully deployed server that has neither. Such a report
 * gets believed, and that is the whole of the trouble.
 */

import { useLang, useT } from "../../shared/i18n";
import { renderDetail } from "../../shared/i18n/render";
import type { DeployStepStatus, PlannedStep } from "../../shared/contract";

/** What to call the state a step is in. */
function mark(status: DeployStepStatus, words: Record<string, string>): string {
  if (status === "Applied") return words.stepApplied;
  if (status === "NotApplied") return words.stepToDo;
  if (typeof status === "object" && "Failed" in status) return words.stepFailed;
  if (typeof status === "object" && "Skipped" in status) {
    const why = status.Skipped.why;
    return why === "NotNeeded" ? words.stepNotNeeded : words.stepNotHere;
  }
  return words.stepToDo;
}

/** What exactly went wrong, when something did. */
function detailOf(status: DeployStepStatus): string | null {
  if (typeof status === "object" && "Failed" in status) return status.Failed.detail;
  if (typeof status === "object" && "Skipped" in status && typeof status.Skipped.why === "object") {
    return status.Skipped.why.NotPossibleHere.detail;
  }
  return null;
}

export function StepList({ steps }: { steps: PlannedStep[] }) {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.deploy as unknown as Record<string, string>;
  const names = t.ui.deploySteps as unknown as Record<string, string>;

  return (
    <ol>
      {steps.map((step) => {
        const detail = detailOf(step.status);
        return (
          <li key={step.id}>
            <span>{names[step.id] ?? step.id}</span>
            <span>{mark(step.status, words)}</span>
            {/* The reason beside its step rather than at the end of the list: by the end of
                the list there is nothing left to attach it to. */}
            {detail && <small>{detail}</small>}
            {/* ⚠ **What this step will actually do** (T507, FR-122). The core has always
                worked it out — the packages by name, the ports, the files, the size of the
                swap file — and the interface typed the field `unknown[]` and read it nowhere,
                showing fifteen general headings instead. That is precisely what this file's
                own comment says a person is not owed.
                Only where there is something to say: a step that only looks says so, and a
                step whose changes are empty (keeping IPv6, or swap that is not needed) says
                nothing rather than showing an empty list. */}
            {step.changes.length > 0 && (
              <ul className="step__changes">
                {step.changes.map((change, i) => (
                  <li key={i}>{renderDetail(change, t, lang)}</li>
                ))}
              </ul>
            )}
          </li>
        );
      })}
    </ol>
  );
}
