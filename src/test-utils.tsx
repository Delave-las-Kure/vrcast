/**
 * Rendering a screen in a chosen language.
 *
 * Every screen now reads its words from a catalogue, so every test has to say which
 * language it is testing. Saying it explicitly rather than letting the environment
 * decide matters: `navigator.language` differs between a developer's machine and CI,
 * and a test that passed here would fail there for a reason nobody could see.
 *
 * The default is Russian because that is the language the wordings were written in and
 * the one most assertions read against.
 */

import { render, type RenderOptions, type RenderResult } from "@testing-library/react";
import type { ReactElement, ReactNode } from "react";
import { LanguageProvider } from "./shared/i18n";
import type { Lang } from "./shared/i18n";

export { ru } from "./shared/i18n/ru";
export { en } from "./shared/i18n/en";

export function renderIn(
  ui: ReactElement,
  lang: Lang = "ru",
  options?: Omit<RenderOptions, "wrapper">,
): RenderResult {
  const wrapper = ({ children }: { children: ReactNode }) => (
    <LanguageProvider initial={lang}>{children}</LanguageProvider>
  );
  return render(ui, { wrapper, ...options });
}
