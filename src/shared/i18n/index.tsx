/**
 * Language of the interface: Russian and English (FR-106).
 *
 * The choice lives in the core, beside every other setting (T324), and survives a
 * restart with them. The default is taken from the system: someone who never opens
 * the settings should still get their own language on the first run.
 *
 * The core does not compose prose any more. It returns a code and the values to
 * substitute; every wording lives here, in one catalogue per language. That keeps
 * the property FR-105 was written for — one situation is explained the same way on
 * every screen — while allowing the wording to exist in two languages at once.
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { ru } from "./ru";
import { en } from "./en";
import type { Catalogue, Lang } from "./catalogue";
import { useSettings } from "../../app/settings";

const CATALOGUES: Record<Lang, Catalogue> = { ru, en };

/** Languages offered in the settings, in the order they are shown. */
export const LANGUAGES: { lang: Lang; label: string }[] = [
  { lang: "ru", label: "Русский" },
  { lang: "en", label: "English" },
];

interface LanguageContextValue {
  lang: Lang;
  setLang: (l: Lang) => void;
  catalogue: Catalogue;
}

const LanguageContext = createContext<LanguageContextValue | null>(null);

function isLang(v: unknown): v is Lang {
  return v === "ru" || v === "en";
}

/**
 * The chosen language, as the core stores it (T324).
 *
 * **Not `localStorage`.** It used to be, while the core kept a `language` field of its own
 * that nobody read — two stores of one choice, which diverge silently and cannot be mended
 * by the person, who does not know about the second one.
 */
function chosen(settings: { language: string | null } | null): Lang | null {
  const v = settings?.language;
  return isLang(v) ? v : null;
}

/**
 * What the system suggests.
 *
 * Only the primary subtag is examined: `ru-RU`, `ru-BY` and plain `ru` all mean the
 * same thing here, and anything else falls back to English rather than guessing.
 */
export function systemLang(): Lang {
  if (typeof navigator === "undefined") return "en";
  const tags = navigator.languages?.length ? navigator.languages : [navigator.language];
  for (const tag of tags) {
    if (typeof tag === "string" && tag.toLowerCase().startsWith("ru")) return "ru";
  }
  return "en";
}

export function LanguageProvider({
  children,
  initial,
}: {
  children: ReactNode;
  /** Used by tests to pin a language instead of depending on the environment. */
  initial?: Lang;
}) {
  const { settings, update } = useSettings();
  const [fallback] = useState<Lang>(() => initial ?? systemLang());

  // Pinned by a test wins; then the person's choice out of the core; then the system's.
  // Until the core has answered, the system's — showing English to somebody who chose
  // Russian for a moment is better than holding the whole window blank while a database
  // is read.
  const lang: Lang = initial ?? chosen(settings) ?? fallback;

  const setLang = useCallback((l: Lang) => update({ language: l }), [update]);

  // The document language is what screen readers and the browser's own spelling rules
  // go by; leaving it at the build-time default would have English text announced with
  // Russian pronunciation, and the other way round.
  useEffect(() => {
    if (typeof document !== "undefined") document.documentElement.lang = lang;
  }, [lang]);

  const value = useMemo<LanguageContextValue>(
    () => ({ lang, setLang, catalogue: CATALOGUES[lang] }),
    [lang, setLang],
  );

  return <LanguageContext.Provider value={value}>{children}</LanguageContext.Provider>;
}

function useLanguageContext(): LanguageContextValue {
  const ctx = useContext(LanguageContext);
  if (!ctx) {
    // Silently falling back to Russian here would hide a missing provider until a
    // screen showed the wrong language to a user. Better to fail while testing.
    throw new Error("useLang/useT used outside LanguageProvider");
  }
  return ctx;
}

/** The current language and how to change it. */
export function useLang(): { lang: Lang; setLang: (l: Lang) => void } {
  const { lang, setLang } = useLanguageContext();
  return { lang, setLang };
}

/** The whole catalogue of the current language. */
export function useT(): Catalogue {
  return useLanguageContext().catalogue;
}

export type { Lang, Catalogue } from "./catalogue";
