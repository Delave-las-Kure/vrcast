/**
 * Putting values into a wording.
 *
 * The core sends a code and raw values; the template that receives them lives in the
 * catalogue of the chosen language. A template says `{name}` for a value as it stands,
 * and `{name|how}` when it needs formatting:
 *
 * - `{bytes|bytes}` — a size, in the units and separator of the language
 * - `{bps|bitrate}` — a bitrate, likewise
 * - `{n|plural:file}` — the word form matching the count
 * - `{name|encoder}` — the human name of a hardware encoder
 *
 * A missing value is left as the literal `{name}` rather than replaced with nothing:
 * a sentence with a visible gap gets reported, and a sentence quietly missing its
 * number reads as complete and says the wrong thing.
 */

import type { AppError, Detail, DetailCode } from "../contract";
import type { Catalogue, Lang } from "./catalogue";
import { formatBitrate, formatBytes, plural } from "./format";
import type { PluralWord } from "./types";

/**
 * Names of hardware encoders as a person would recognise them.
 *
 * Kept beside the renderer rather than in each catalogue: `h264_nvenc` is "NVIDIA" in
 * every language, and only the surrounding words change. An unknown encoder falls back
 * to its ffmpeg name — better an unfamiliar word than a blank.
 */
const ENCODER_NAMES: Record<Lang, Record<string, string>> = {
  ru: {
    h264_nvenc: "видеокарты NVIDIA",
    h264_qsv: "встроенной графики Intel",
    h264_amf: "видеокарты AMD",
    h264_vaapi: "видеокарты (VAAPI)",
  },
  en: {
    h264_nvenc: "an NVIDIA graphics card",
    h264_qsv: "Intel integrated graphics",
    h264_amf: "an AMD graphics card",
    h264_vaapi: "a graphics card (VAAPI)",
  },
};

const PLACEHOLDER = /\{(\w+)(?:\|([a-z]+)(?::(\w+))?)?\}/g;

export function fill(
  template: string,
  params: Record<string, string | number> | undefined,
  catalogue: Catalogue,
  lang: Lang,
): string {
  return template.replace(PLACEHOLDER, (whole, name: string, how?: string, arg?: string) => {
    const value = params?.[name];
    if (value === undefined || value === null) return whole;

    switch (how) {
      case "bytes":
        return formatBytes(Number(value), lang);
      case "bitrate":
        return formatBitrate(Number(value), lang);
      case "plural": {
        const forms = catalogue.plurals[arg as PluralWord];
        return forms ? plural(Number(value), forms, lang) : String(value);
      }
      case "encoder": {
        const key = String(value);
        return ENCODER_NAMES[lang][key] ?? key;
      }
      default:
        return String(value);
    }
  });
}

/** One thing the core wanted said, in the current language. */
export function renderDetail(detail: Detail, catalogue: Catalogue, lang: Lang): string {
  const template = catalogue.details[detail.key];
  // An unknown key means a core newer than this interface. Showing the key beats
  // showing nothing: it is searchable, and silence looks like everything is fine.
  if (!template) return detail.key;
  return fill(template, detail.params, catalogue, lang);
}

/** Every detail of an error, in order, as one paragraph. */
export function renderDetails(
  details: Detail[] | undefined,
  catalogue: Catalogue,
  lang: Lang,
): string {
  if (!details || details.length === 0) return "";
  return details.map((d) => renderDetail(d, catalogue, lang)).join(" ");
}

/** What to show for an error: what happened, and what to do about it. */
export function renderError(
  error: AppError,
  catalogue: Catalogue,
  lang: Lang,
): { message: string; hint: string } {
  const wording = catalogue.errors[error.code];
  const spelled = renderDetails(error.details, catalogue, lang);
  return {
    // The details are more specific than the code's own message, so they replace it
    // when present. The hint always comes from the code: it says what to do, and that
    // does not change with the particulars.
    message: spelled || wording?.message || error.code,
    hint: wording?.hint ?? "",
  };
}

/** A stage name beside a running task. */
export function renderStage(
  stage: DetailCode | null,
  catalogue: Catalogue,
  lang: Lang,
): string {
  if (!stage) return "";
  return fill(catalogue.details[stage] ?? stage, undefined, catalogue, lang);
}
