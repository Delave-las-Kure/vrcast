/**
 * T059 — правила показа чисел.
 *
 * Проверяется не «функция что-то вернула», а то, на чём легко ошибиться и что
 * пользователь заметит сразу: склонение после числа, неизвестное против нуля
 * и перевод байтов в человеческий вид.
 */

import { describe, expect, it } from "vitest";
import {
  countOf,
  formatBitrate,
  formatBytes,
  formatDuration,
  formatResolution,
  plural,
  usedFraction,
} from "../format";

describe("объём", () => {
  it("переводит байты в понятные единицы", () => {
    expect(formatBytes(512)).toBe("512 Б");
    expect(formatBytes(4096)).toBe("4,0 КБ");
    expect(formatBytes(1024 * 1024 * 3.5)).toBe("3,5 МБ");
    expect(formatBytes(1024 ** 3 * 12)).toBe("12,0 ГБ");
  });

  it("ноль показывает нулём, а неизвестное — прочерком", () => {
    // Разница существенная: «0 Б» значит «файл пуст», а «—» — «мы не знаем».
    expect(formatBytes(0)).toBe("0 Б");
    expect(formatBytes(null)).toBe("—");
    expect(formatBytes(undefined)).toBe("—");
  });
});

describe("длительность", () => {
  it("показывает часы только когда они есть", () => {
    expect(formatDuration(125)).toBe("2:05");
    expect(formatDuration(3725)).toBe("1:02:05");
  });

  it("неизвестную и нулевую показывает прочерком", () => {
    expect(formatDuration(null)).toBe("—");
    expect(formatDuration(0)).toBe("—");
  });
});

describe("битрейт", () => {
  it("переводит в мегабиты, а совсем малое — в килобиты", () => {
    expect(formatBitrate(9_000_000)).toBe("9,0 Мбит/с");
    expect(formatBitrate(320_000)).toBe("320 кбит/с");
  });

  it("неизвестный показывает прочерком", () => {
    expect(formatBitrate(null)).toBe("—");
  });
});

describe("разрешение", () => {
  it("показывает пару чисел, а при неполных данных — прочерк", () => {
    expect(formatResolution(3840, 2160)).toBe("3840×2160");
    // Половина разрешения бесполезна и вводит в заблуждение.
    expect(formatResolution(1920, null)).toBe("—");
    expect(formatResolution(null, null)).toBe("—");
  });
});

describe("склонение после числа", () => {
  it("следует правилам русского языка", () => {
    expect(countOf(1, "файл", "файла", "файлов")).toBe("1 файл");
    expect(countOf(2, "файл", "файла", "файлов")).toBe("2 файла");
    expect(countOf(5, "файл", "файла", "файлов")).toBe("5 файлов");
    expect(countOf(21, "файл", "файла", "файлов")).toBe("21 файл");
  });

  it("не спотыкается на второй десятке", () => {
    // Здесь ошибаются чаще всего: одиннадцать оканчивается на единицу,
    // но это «файлов», а не «файл».
    expect(plural(11, "файл", "файла", "файлов")).toBe("файлов");
    expect(plural(12, "файл", "файла", "файлов")).toBe("файлов");
    expect(plural(14, "файл", "файла", "файлов")).toBe("файлов");
    expect(plural(111, "файл", "файла", "файлов")).toBe("файлов");
  });
});

describe("занятое место", () => {
  it("считает долю и не выходит за границы", () => {
    expect(usedFraction(100, 25)).toBeCloseTo(0.75);
    expect(usedFraction(100, 0)).toBe(1);
    // Пустой или испорченный ответ сервера не должен рисовать полосу шириной −300 %.
    expect(usedFraction(0, 0)).toBe(0);
    expect(usedFraction(100, 200)).toBe(0);
  });
});
