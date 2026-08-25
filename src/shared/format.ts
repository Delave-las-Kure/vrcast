/**
 * Как показывать числа человеку.
 *
 * Собрано в одном месте намеренно: объём файла, посчитанный на одном экране в
 * гигабайтах, а на другом в гибибайтах, — это не мелочь, а расхождение, из-за
 * которого пользователь перестаёт верить обоим.
 *
 * Разделитель дробной части берётся из русской раскладки — запятая, как принято.
 */

const ЕДИНИЦЫ_ОБЪЁМА = ["Б", "КБ", "МБ", "ГБ", "ТБ"] as const;

/** Объём: 4096 → «4,0 КБ». Ноль показывается нулём, а не прочерком. */
export function formatBytes(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined || Number.isNaN(bytes)) return "—";
  if (bytes < 1024) return `${Math.round(bytes)} ${ЕДИНИЦЫ_ОБЪЁМА[0]}`;

  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit + 1 < ЕДИНИЦЫ_ОБЪЁМА.length) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(1).replace(".", ",")} ${ЕДИНИЦЫ_ОБЪЁМА[unit]}`;
}

/** Длительность: 3725 → «1:02:05». Меньше часа — без ведущих часов. */
export function formatDuration(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined || seconds <= 0) return "—";

  const total = Math.round(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const two = (n: number) => String(n).padStart(2, "0");

  return h > 0 ? `${h}:${two(m)}:${two(s)}` : `${m}:${two(s)}`;
}

/** Битрейт: 9_000_000 → «9,0 Мбит/с». */
export function formatBitrate(bps: number | null | undefined): string {
  if (bps === null || bps === undefined || bps <= 0) return "—";
  const mbit = bps / 1_000_000;
  if (mbit < 1) return `${Math.round(bps / 1000)} кбит/с`;
  return `${mbit.toFixed(1).replace(".", ",")} Мбит/с`;
}

/** Разрешение: 3840×2160. Прочерк, если хотя бы одна сторона неизвестна. */
export function formatResolution(
  width: number | null | undefined,
  height: number | null | undefined,
): string {
  if (!width || !height) return "—";
  return `${width}×${height}`;
}

/**
 * Слово в нужном числе: 1 файл, 2 файла, 5 файлов.
 *
 * Правило неочевидное: одиннадцать — «файлов», а не «файл», хотя оканчивается
 * на единицу. Без разбора двух последних цифр получается «11 файл».
 */
export function plural(n: number, one: string, few: string, many: string): string {
  const lastTwo = n % 100;
  const last = n % 10;
  if (lastTwo >= 11 && lastTwo <= 14) return many;
  if (last === 1) return one;
  if (last >= 2 && last <= 4) return few;
  return many;
}

/** «3 файла», «1 медиа», «12 файлов». */
export function countOf(n: number, one: string, few: string, many: string): string {
  return `${n} ${plural(n, one, few, many)}`;
}

/** Доля занятого места, от 0 до 1. */
export function usedFraction(total: number, free: number): number {
  if (total <= 0) return 0;
  return Math.min(1, Math.max(0, (total - free) / total));
}
