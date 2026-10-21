// `as const`: an array of values of one type stays an array (it is used as one); one of a key
// and a value, `[k, v] as const`, is a tuple (new Map takes them); Math.min() and Math.max().
const COLORS = ["red", "green", "blue"] as const
type Color = (typeof COLORS)[number]
const PAIR = [1, 2] as const
function isColor(s: string): boolean { return (COLORS as readonly string[]).includes(s) }
for (const c of COLORS) console.log(c)
console.log(COLORS.length, COLORS[1], COLORS.map((c) => c.toUpperCase()), COLORS.includes("red"), COLORS.indexOf("blue"), isColor("x"))
let i = 0
console.log(COLORS[i], PAIR[0] + PAIR[1], [...COLORS].join("+"))
const c: Color = "green"
console.log(c)
type Task = { id: string; d: number }
const tasks: Task[] = [{ id: "a", d: 1 }, { id: "b", d: 2 }]
const t1 = ["x", 1] as const
const byId = new Map(tasks.map((t) => [t.id, t] as const))
const byId2 = new Map(tasks.map((t): [string, Task] => [t.id, t]))
console.log(byId.get("b")!.d, byId2.size, t1[0], Math.min(), Math.max())
