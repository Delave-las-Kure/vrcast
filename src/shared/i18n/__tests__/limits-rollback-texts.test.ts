/**
 * T646 (QA-22 №6) — the texts of `LIMITS_ROLLBACK_FAILED` say no more about the server than
 * the core knows.
 *
 * - `limits_list` reads the rules file, not the configuration Caddy is running: after a failed
 *   change the two may differ (T628), so the hint must not promise "what is in force now";
 * - `LIMITS_ROLLBACK_NOT_STARTED` means the end of the change's previous command was not
 *   confirmed — the command may have finished, or the barrier itself failed to run — not that
 *   it is still running;
 * - a change that failed before the swap left no new rules in the file: they *may* have stayed,
 *   only if the change got as far as replacing the file.
 */

import { describe, expect, it } from "vitest";
import { en } from "../en";
import { ru } from "../ru";

describe("the texts of a rollback that did not happen (T646)", () => {
  it("ru: the list is the rules file, the end is unconfirmed, the rules only may have stayed", () => {
    const { message, hint } = ru.errors.LIMITS_ROLLBACK_FAILED;
    const notStarted = ru.details.LIMITS_ROLLBACK_NOT_STARTED;

    expect(message).toContain("не подтверждён");
    expect(hint).toContain("файле правил");
    expect(hint).toContain("может не совпадать");
    expect(hint).not.toContain("что действует на сервере сейчас");

    expect(notStarted).toContain("окончание предыдущей команды");
    expect(notStarted).toContain("не подтверждено");
    expect(notStarted).toContain("дошло до замены");
    expect(notStarted).toContain("могли остаться");
    expect(notStarted).not.toContain("ещё идёт");
    expect(notStarted).not.toContain("остались в файле");
  });

  it("en: the list is the rules file, the end is unconfirmed, the rules only may have stayed", () => {
    const { message, hint } = en.errors.LIMITS_ROLLBACK_FAILED;
    const notStarted = en.details.LIMITS_ROLLBACK_NOT_STARTED;

    expect(message).toContain("not confirmed");
    expect(hint).toContain("rules file");
    expect(hint).toContain("may differ");
    expect(hint).not.toContain("really in force");

    expect(notStarted).toContain("was not confirmed");
    expect(notStarted).toContain("got as far as replacing");
    expect(notStarted).toContain("may have stayed");
    expect(notStarted).not.toContain("is still running");
    expect(notStarted).not.toContain("stayed in the file and come");
  });

  it("the unsuccessful rollback still sends a person to look, in both languages", () => {
    expect(ru.details.LIMITS_ROLLBACK_UNSUCCESSFUL).toContain("может не работать");
    expect(en.details.LIMITS_ROLLBACK_UNSUCCESSFUL).toContain("may not be working");
  });
});
