/**
 * The shape a catalogue has to fill.
 *
 * Errors and details are typed as records over the contract's own unions, so a code
 * added in the core and forgotten here fails the build. That is deliberate: FR-105
 * used to be guarded by a test asserting every code had a Russian message and hint;
 * with two languages the compiler can do it, and it does it for both at once.
 */

import type { DetailCode, ErrorCode } from "../contract";

export type Lang = "ru" | "en";

/** What happened, and what to do about it. Both are required — see FR-105. */
export interface Wording {
  message: string;
  hint: string;
}

/**
 * Word forms for counting.
 *
 * Russian needs three (файл / файла / файлов) and English two. Both are declared so
 * neither language has to pretend it works like the other.
 */
export interface PluralForms {
  one: string;
  few: string;
  many: string;
}

/**
 * Things that get counted. Latin keys in both languages: the key is referenced from
 * a template, and a Russian key would leave the English catalogue naming a Russian
 * word for no reason.
 */
export type PluralWord = "file" | "media" | "time" | "task" | "track";

export interface CatalogueCore {
  /** Wording for every error code. */
  errors: Record<ErrorCode, Wording>;
  /** Wording for every detail the core can say. */
  details: Record<DetailCode, string>;
  /** Word forms used by the `plural` substitution. */
  plurals: Record<PluralWord, PluralForms>;
}

/**
 * Widen literal types back to `string`.
 *
 * The Russian catalogue is the source of the shape, and without this every English
 * entry would have to repeat the Russian text to satisfy its literal type. Applied
 * recursively so the interface section can be grouped into objects.
 */
export type Widen<T> = {
  [K in keyof T]: T[K] extends string ? string : Widen<T[K]>;
};
