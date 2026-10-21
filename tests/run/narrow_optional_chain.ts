// Optional chains narrow (`t?.type === "op"` holds: t is there), and strings cast to literal types.
type Tok = { type: "op"; value: string } | { type: "num"; n: number }
function show(t: Tok | undefined): string {
  if (t?.type === "op" && t.value === "+") return "plus"
  if (t?.type === "num") return `n${t.n}`
  return t === undefined ? "none" : "other"
}
console.log(show({ type: "op", value: "+" }), show({ type: "num", n: 3 }), show(undefined), show({ type: "op", value: "-" }))
type Op = "+" | "-" | "*"
const xs: Op[] = []
for (const ch of "+*-") xs.push(ch as Op)
console.log(xs, xs[0] === "+", (("-") as Op))
