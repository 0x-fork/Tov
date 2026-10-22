type Pt = { x: number }
function area(w: number, h: number) {
  type Box = { w: number; h: number }
  interface Named { name: string }
  const b: Box = { w, h }
  const n: Named = { name: "box" }
  return `${n.name} ${b.w * b.h}`
}
function other() {
  type Box = string[]
  const b: Box = ["a", "b"]
  return b.join("+")
}
function tree() {
  type Tree = { v: number; kids: Tree[] }
  const t: Tree = { v: 1, kids: [{ v: 2, kids: [] }] }
  const sum = (n: Tree): number => n.v + n.kids.reduce((a, k) => a + sum(k), 0)
  return sum(t)
}
const p: Pt = { x: 1 }
console.log(area(2, 3), other(), tree(), p.x)
