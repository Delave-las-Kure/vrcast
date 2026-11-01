/**
 * Tour 0.3.0 (F08, D05) — the program's own answer under «Подробнее» is set apart from the
 * explanation: a label of its own and a monospace block, in both languages.
 */

import { fireEvent, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { en, renderIn, ru } from "../../../test-utils";
import type { AppError } from "../../../shared/contract";
import { ErrorNotice } from "../ErrorNotice";

const broken: AppError = {
  code: "DOMAIN_NOT_SERVING",
  details: [],
  cause: "moov atom not found\nInvalid data found when processing input",
};

describe("the technical answer in an error", () => {
  it.each([
    ["ru", ru.ui.common.technical, "Ответ программы:"],
    ["en", en.ui.common.technical, "Technical details:"],
  ] as const)("in %s: under its own label, in a block of its own", (lang, label, expected) => {
    expect(label).toBe(expected);
    const { container } = renderIn(<ErrorNotice error={broken} />, lang);
    fireEvent.click(screen.getByText(lang === "ru" ? ru.ui.common.more : en.ui.common.more));
    expect(screen.getByText(label)).toBeInTheDocument();
    const block = container.querySelector("pre.error-more__cause-text");
    expect(block?.textContent).toBe(broken.cause);
    // Not run on after the explanation as one paragraph.
    expect(container.querySelector("p.error-more__cause")).toBeNull();
  });

  it("says nothing of the kind when there is no technical answer", () => {
    const { container } = renderIn(<ErrorNotice error={{ ...broken, cause: undefined }} />, "ru");
    expect(container.querySelector(".error-more__cause")).toBeNull();
    expect(screen.queryByText(ru.ui.common.technical)).toBeNull();
  });
});
