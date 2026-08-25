/**
 * The catalogues themselves.
 *
 * The compiler already guarantees that both languages have the same keys — the English
 * one is typed by the Russian one. What it cannot see is an entry that exists but says
 * nothing, a substitution named in one language and forgotten in the other, or a
 * count that reads "11 файл". Those are checked here.
 *
 * Why it matters that these are real checks: an empty string still satisfies `string`,
 * and a screen showing a blank where an explanation should be looks like a bug in the
 * server rather than a hole in a catalogue.
 */

import { describe, expect, it } from "vitest";
import { ru } from "../ru";
import { en } from "../en";
import { fill, renderDetail, renderError } from "../render";
import { formatBytes, plural } from "../format";
import type { Catalogue, Lang } from "../catalogue";

const CATALOGUES: Array<[Lang, Catalogue]> = [
  ["ru", ru],
  ["en", en],
];

/** Every `{name}` and `{name|how}` a template asks for. */
function placeholders(template: string): string[] {
  return [...template.matchAll(/\{(\w+)(?:\|[a-z]+(?::\w+)?)?\}/g)].map((m) => m[1]);
}

describe("catalogues", () => {
  it.each(CATALOGUES)("%s says something for every error code", (_lang, catalogue) => {
    for (const [code, wording] of Object.entries(catalogue.errors)) {
      expect(wording.message.trim(), `message for ${code}`).not.toBe("");
      // The hint is not decoration: a message without one leaves a person alone with
      // a problem they know nothing about. FR-105 demands both, in every language.
      expect(wording.hint.trim(), `hint for ${code}`).not.toBe("");
    }
  });

  it.each(CATALOGUES)("%s says something for every detail", (_lang, catalogue) => {
    for (const [code, template] of Object.entries(catalogue.details)) {
      expect(template.trim(), `detail ${code}`).not.toBe("");
    }
  });

  it("asks for the same substitutions in both languages", () => {
    // A number named in one language and forgotten in the other is the failure this
    // catches: the sentence still reads, and it silently says something else.
    for (const code of Object.keys(ru.details) as Array<keyof typeof ru.details>) {
      const inRu = placeholders(ru.details[code]).sort();
      const inEn = placeholders(en.details[code]).sort();
      expect(inEn, `substitutions of ${code}`).toEqual(inRu);
    }
  });

  it("counts in Russian the way Russian counts", () => {
    const forms = ru.plurals.file;
    expect(plural(1, forms, "ru")).toBe("файл");
    expect(plural(2, forms, "ru")).toBe("файла");
    expect(plural(5, forms, "ru")).toBe("файлов");
    // Eleven ends in a one and still takes «файлов». Examining only the last digit
    // gives "11 файл", which is the mistake this rule exists to prevent.
    expect(plural(11, forms, "ru")).toBe("файлов");
    expect(plural(21, forms, "ru")).toBe("файл");
  });

  it("counts in English the way English counts", () => {
    const forms = en.plurals.file;
    expect(plural(1, forms, "en")).toBe("file");
    expect(plural(11, forms, "en")).toBe("files");
  });

  it("writes sizes in the units and separator of each language", () => {
    expect(formatBytes(4096, "ru")).toBe("4,0 КБ");
    expect(formatBytes(4096, "en")).toBe("4.0 KB");
  });
});

describe("filling a wording in", () => {
  it("puts a size in as a size, not as a raw number of bytes", () => {
    const error = {
      code: "REMOTE_DISK_FULL" as const,
      details: [
        {
          key: "NOT_ENOUGH_SPACE" as const,
          params: { short_by: 23_622_320_128, needed: 32_212_254_720, free: 8_589_934_592 },
        },
      ],
    };
    expect(renderError(error, ru, "ru").message).toContain("22,0 ГБ");
    expect(renderError(error, en, "en").message).toContain("22.0 GB");
  });

  it("leaves a gap visible when a value is missing", () => {
    // Replacing it with nothing would leave a sentence that reads as complete and
    // says the wrong thing. A visible `{free}` gets reported.
    const shown = fill("short by {short_by|bytes}, {free} free", { short_by: 1024 }, ru, "ru");
    expect(shown).toContain("{free}");
  });

  it("names a hardware encoder the way a person would recognise it", () => {
    const detail = {
      key: "NOTICE_HARDWARE_FAILED" as const,
      params: { encoder: "h264_nvenc" },
    };
    expect(renderDetail(detail, ru, "ru")).toContain("NVIDIA");
    expect(renderDetail(detail, en, "en")).toContain("NVIDIA");
  });

  it("falls back to the ffmpeg name for an encoder nobody has named", () => {
    const detail = {
      key: "NOTICE_HARDWARE_FAILED" as const,
      params: { encoder: "h264_something_new" },
    };
    expect(renderDetail(detail, en, "en")).toContain("h264_something_new");
  });

  it("shows the code itself when the core knows a detail this interface does not", () => {
    // A newer core against an older interface. The key is searchable; silence is not.
    const detail = { key: "SOMETHING_ADDED_LATER" as never };
    expect(renderDetail(detail, ru, "ru")).toBe("SOMETHING_ADDED_LATER");
  });

  it("prefers what the core said specifically over the code's general message", () => {
    const general = ru.errors.INVALID_INPUT.message;
    const specific = renderError(
      { code: "INVALID_INPUT", details: [{ key: "PROFILE_HOST_EMPTY" }] },
      ru,
      "ru",
    );
    expect(specific.message).not.toBe(general);
    expect(specific.message).toBe(ru.details.PROFILE_HOST_EMPTY);
    // The hint still comes from the code: it says what to do, and that does not
    // change with the particulars.
    expect(specific.hint).toBe(ru.errors.INVALID_INPUT.hint);
  });

  it("says everything the core asked for, in order", () => {
    const shown = renderError(
      {
        code: "NAME_EXISTS",
        details: [
          { key: "NAME_WILL_BE_REPLACED", params: { name: "film.mp4" } },
          { key: "CDN_KEEPS_OLD_COPY" },
        ],
      },
      ru,
      "ru",
    );
    expect(shown.message).toContain("film.mp4");
    expect(shown.message).toContain("CDN");
  });
});

describe("what is said about falling back to the processor", () => {
  // These used to be checked in the core, beside the code that chose the encoder.
  // The wording moved here, so the checks moved with it — otherwise the measurement
  // behind them would be lost the first time someone rewrote a sentence.

  it("says the cost is time, not quality", () => {
    // Measured 2026-08-02: software x264 against NVENC gives +1.13 VMAF at four
    // megabits and nothing at working bitrates. Frightening people about quality
    // would be a lie; what they actually lose is hours.
    expect(ru.details.NOTICE_NO_HARDWARE_FOUND).toContain("не пострадает");
    expect(ru.details.NOTICE_NO_HARDWARE_FOUND).toMatch(/больше|час/);
    expect(en.details.NOTICE_NO_HARDWARE_FOUND).toContain("not suffer");
    expect(en.details.NOTICE_NO_HARDWARE_FOUND).toMatch(/longer|hour/);
  });

  it("does not dress a deliberate choice up as a misfortune", () => {
    // Someone who asked for the processor is told their wish was carried out, not
    // that something went wrong. Same code, same wording, would say the opposite.
    expect(ru.details.NOTICE_SOFTWARE_AS_ASKED).toContain("как вы и просили");
    expect(en.details.NOTICE_SOFTWARE_AS_ASKED).toContain("as you asked");
    expect(ru.details.NOTICE_SOFTWARE_AS_ASKED).not.toBe(
      ru.details.NOTICE_NO_HARDWARE_FOUND,
    );
  });

  it("keeps the machine name out of the sentence a person reads", () => {
    const said = renderDetail(
      { key: "NOTICE_HARDWARE_FAILED", params: { encoder: "h264_nvenc" } },
      ru,
      "ru",
    );
    expect(said).not.toContain("h264_nvenc");
    expect(said).toContain("NVIDIA");
  });
});
