// An array of object literals with the same fields is of one record type, its numbers joined:
// `[{ year: 0 }, { year: 12.2 }]` is a `{ year: number }[]`, as in TypeScript.

const weekday = (year: number, month: number): number => (year * 12 + month) % 7;
const years = [{ year: 0, month: 1 }, { year: 12.2, month: 1 }];
const months = [{ year: 2001, month: -5 }, { year: 2001, month: 9.3 }];
for (const { year, month } of years) console.log(weekday(year, month));
for (const r of months) console.log(weekday(r.year, r.month));
const nested = [{ at: { x: 1 }, tags: ["a"] }, { at: { x: 2.5 }, tags: [] as string[] }];
console.log(nested.map((n) => n.at.x * 2), nested);
