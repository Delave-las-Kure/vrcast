/**
 * Шаги развёртывания списком — и до согласия, и по ходу (FR-122, FR-123).
 *
 * Один список на оба случая намеренно. Человек, который только что читал, что будет сделано,
 * смотрит на то же самое, пока оно делается: те же шаги, в том же порядке, с меняющимися
 * пометками. Два разных списка заставляли бы искать глазами, где он остановился.
 *
 * **«Здесь не установить» — не «сделано».** Это ответ проверки, а не её отсутствие: в
 * контейнере нельзя ни включить подкачку, ни тронуть настройки ядра, и прогон, свернувший это
 * в «готово», отчитался бы о полностью развёрнутом сервере, у которого нет ни того ни
 * другого. Такому отчёту верят, и в этом вся беда.
 */

import { useT } from "../../shared/i18n";
import type { DeployStepStatus, PlannedStep } from "../../shared/contract";

/** Как назвать состояние шага. */
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

/** Что именно пошло не так, когда что-то пошло не так. */
function detailOf(status: DeployStepStatus): string | null {
  if (typeof status === "object" && "Failed" in status) return status.Failed.detail;
  if (typeof status === "object" && "Skipped" in status && typeof status.Skipped.why === "object") {
    return status.Skipped.why.NotPossibleHere.detail;
  }
  return null;
}

export function StepList({ steps }: { steps: PlannedStep[] }) {
  const t = useT();
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
            {/* Причина рядом с шагом, а не в конце списка: к концу списка её уже не с чем
                связать. */}
            {detail && <small>{detail}</small>}
          </li>
        );
      })}
    </ol>
  );
}
