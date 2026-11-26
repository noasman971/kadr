import { describe, expect, it } from "vitest";

import { formatEditableNumber, parseEditableNumber } from "./numberText";

describe("formatEditableNumber", () => {
  it("writes thousands without grouping in English and French", () => {
    expect(formatEditableNumber(1200, 0, "en")).toBe("1200");
    expect(formatEditableNumber(1200, 0, "fr")).toBe("1200");
    expect(formatEditableNumber(123456.78, 2, "en")).toBe("123456.78");
  });

  it("uses each language's decimal mark", () => {
    expect(formatEditableNumber(17.6, 1, "en")).toBe("17.6");
    expect(formatEditableNumber(17.6, 1, "fr")).toBe("17,6");
  });
});

describe("parseEditableNumber", () => {
  it("reads back what the field shows, in both languages", () => {
    const values = [0, 0.05, 1, -12.5, 999, 1000, 1200, 4096, 123456.78, -3600];
    for (const decimals of [0, 1, 2]) {
      for (const value of values) {
        // Plain JavaScript reads the English text: no grouping, a point as decimal mark.
        const shown = Number(formatEditableNumber(value, decimals, "en"));
        expect(Math.abs(shown - value)).toBeLessThanOrEqual(0.5 / 10 ** decimals);
        for (const language of ["en", "fr"]) {
          expect(parseEditableNumber(formatEditableNumber(value, decimals, language))).toBe(shown);
        }
      }
    }
  });

  it("accepts typed spaces and a comma as the decimal mark", () => {
    expect(parseEditableNumber("1 200")).toBe(1200);
    expect(parseEditableNumber("1 200,5")).toBe(1200.5);
    expect(parseEditableNumber(" 2,5 ")).toBe(2.5);
  });

  it("says when the text is not a number", () => {
    expect(parseEditableNumber("abc")).toBeNaN();
    expect(parseEditableNumber("12px")).toBeNaN();
  });
});
