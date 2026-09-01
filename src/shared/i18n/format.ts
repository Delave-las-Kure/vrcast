/**
 * How numbers are shown, in each language.
 *
 * Gathered in one place on purpose: a file size counted in gigabytes on one screen and
 * gibibytes on another is not a detail — it is a discrepancy that stops the user
 * believing either of them.
 *
 * Both languages share the arithmetic and differ only in the separator and the unit
 * names, so the two can never drift apart in the part that matters.
 */

import type { Lang, PluralForms } from "./types";

const SIZE_UNITS: Record<Lang, readonly string[]> = {
  ru: ["Б", "КБ", "МБ", "ГБ", "ТБ"],
  en: ["B", "KB", "MB", "GB", "TB"],
};

const BITRATE_UNITS: Record<Lang, { kbit: string; mbit: string }> = {
  ru: { kbit: "кбит/с", mbit: "Мбит/с" },
  en: { kbit: "kbit/s", mbit: "Mbit/s" },
};

/** Russian writes a comma where English writes a point. */
function decimal(value: number, lang: Lang): string {
  const s = value.toFixed(1);
  return lang === "ru" ? s.replace(".", ",") : s;
}

/** Nothing to show. A dash, not a zero: zero is a value, absence is not. */
const NOTHING = "—";

/** Size: 4096 → «4,0 КБ» / "4.0 KB". Zero shows as zero, not as a dash. */
export function formatBytes(bytes: number | null | undefined, lang: Lang): string {
  if (bytes === null || bytes === undefined || Number.isNaN(bytes)) return NOTHING;
  const units = SIZE_UNITS[lang];
  if (bytes < 1024) return `${Math.round(bytes)} ${units[0]}`;

  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit + 1 < units.length) {
    value /= 1024;
    unit += 1;
  }
  return `${decimal(value, lang)} ${units[unit]}`;
}

/** Duration: 3725 → «1:02:05». Under an hour, no leading hours. Language-neutral. */
export function formatDuration(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined || seconds <= 0) return NOTHING;

  const total = Math.round(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const two = (n: number) => String(n).padStart(2, "0");

  return h > 0 ? `${h}:${two(m)}:${two(s)}` : `${m}:${two(s)}`;
}

/** Bitrate: 9_000_000 → «9,0 Мбит/с» / "9.0 Mbit/s". */
export function formatBitrate(bps: number | null | undefined, lang: Lang): string {
  if (bps === null || bps === undefined || bps <= 0) return NOTHING;
  const units = BITRATE_UNITS[lang];
  const mbit = bps / 1_000_000;
  if (mbit < 1) return `${Math.round(bps / 1000)} ${units.kbit}`;
  return `${decimal(mbit, lang)} ${units.mbit}`;
}

/** Resolution: 3840×2160. A dash if either side is unknown. Language-neutral. */
export function formatResolution(
  width: number | null | undefined,
  height: number | null | undefined,
): string {
  if (!width || !height) return NOTHING;
  return `${width}×${height}`;
}

/**
 * Pick the word form for a count.
 *
 * Russian: «1 файл», «2 файла», «5 файлов» — and eleven takes «файлов» despite ending in a
 * one, which is why the last two digits are examined and not just the last. English
 * has two forms and reaches `many` for everything but one.
 */
export function plural(n: number, forms: PluralForms, lang: Lang): string {
  if (lang === "en") return n === 1 ? forms.one : forms.many;

  const lastTwo = n % 100;
  const last = n % 10;
  if (lastTwo >= 11 && lastTwo <= 14) return forms.many;
  if (last === 1) return forms.one;
  if (last >= 2 && last <= 4) return forms.few;
  return forms.many;
}

/** How full the disk is, from 0 to 1. */
export function usedFraction(total: number, free: number): number {
  if (total <= 0) return 0;
  return Math.min(1, Math.max(0, (total - free) / total));
}
