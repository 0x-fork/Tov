type Dir = "up" | "down" | "left";
const a: Dir = "up";
const b: Dir = "left";
const delta: Record<Dir, number> = { up: -1, down: 1, left: 0 };
const dirs = [a, b];
for (const d of dirs) console.log(d, delta[d]);
const words = ["x", "y"];
words.push("z");
console.log(words, dirs.length);
