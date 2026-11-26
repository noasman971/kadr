// The text of an editable number field. It is written without grouping
// separators, so reading it back gives the same number: with grouping, English
// shows 1200 as "1,200", which reads back as 1.2.

/** A number as an editable field shows it, in the language's style, without grouping. */
export function formatEditableNumber(value: number, maximumFractionDigits: number, language: string): string {
  return new Intl.NumberFormat(language, { maximumFractionDigits, useGrouping: false }).format(value);
}

/**
 * The number typed in a field, or `NaN` if it is not one. Spaces are ignored
 * (French groups digits with them when typing), and a comma is the decimal mark,
 * as in French. An empty field reads as 0, as it always has.
 */
export function parseEditableNumber(text: string): number {
  return Number(text.replace(/\s/g, "").replace(",", "."));
}
