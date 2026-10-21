const user = { id: 7, name: "Zed", role: "admin", meta: { created: 2020 } };
const { name, ...others } = user;
console.log(name, others);
const { id, meta: { created }, ...more } = user;
console.log(id, created, more);
const [first, ...tail] = [1, 2, 3];
console.log(first, tail);

enum Direction {
  Up,
  Down,
  Left,
  Right,
}
function step(d: Direction): number {
  switch (d) {
    case Direction.Up:
      return -10;
    case Direction.Down:
      return 10;
    case Direction.Left:
      return -1;
    case Direction.Right:
      return 1;
  }
}
const path = [Direction.Up, Direction.Right, Direction.Down];
for (const d of path) console.log(Direction[d], step(d));
console.log(Direction[3], Direction.Left);

const transitions = {
  green: { next: "yellow", duration: 30 },
  yellow: { next: "red", duration: 5 },
  red: { next: "green", duration: 25 },
} as const;
type Light = keyof typeof transitions;
let light: Light = "green";
let elapsed = 0;
const timeline: string[] = [];
while (elapsed < 100) {
  timeline.push(`${elapsed}s:${light}`);
  elapsed += transitions[light].duration;
  light = transitions[light].next;
}
console.log(timeline.join(" "));
console.log(JSON.stringify({ a: 1, b: [2] }, null, 2));

async function main() {
  const r = await Promise.reject(new Error("nope")).catch((e: unknown) => (e instanceof Error ? e.message : "?"));
  console.log(r);
}
main();
