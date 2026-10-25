// Predicates that return a value, not a boolean: its truthiness decides, as in JavaScript.
interface Row { id: number; note?: string; score: number; tags: string[]; parent?: Row }
const rows: Row[] = [{ id: 0, score: 0, tags: [] }, { id: 1, note: "", score: NaN, tags: ["a"] }, { id: 2, note: "x", score: 2.5, tags: [] }];
rows.push({ id: 3, score: -1, tags: ["b"], parent: rows[0] });
console.log(rows.filter(r => r.note).map(r => r.id).join(","), rows.filter(r => r.score).map(r => r.id).join(","), rows.filter(r => r.id).length);
console.log(rows.some(r => r.parent), rows.every(r => r.tags), rows.find(r => r.tags.length)?.id, rows.findIndex(r => r.note), rows.filter(r => r.parent?.note).length);
const words = ["", "a", "", "bc"];
console.log(words.filter(w => w).join("|"), [0, 1, 2, 0, 3].filter(n => n).length, [NaN, 1].filter(x => x).length);
