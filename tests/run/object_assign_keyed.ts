interface Todo { id: number; title: string; done: boolean; tag?: string }
type Patch = { title?: string; done?: boolean };
const todos: Todo[] = [{ id: 1, title: "a", done: false }, { id: 2, title: "b", done: true, tag: "x" }];
function patch(id: number, p: Patch): Todo | undefined {
  const t = todos.find((x) => x.id === id);
  if (t) Object.assign(t, p);
  return t;
}
console.log(patch(1, { done: true }), patch(2, { title: "B" }), patch(9, {}));
const base = { x: 1, y: 2 };
const extra = { y: 5 };
const r = Object.assign(base, extra);
console.log(base, r.y, Object.freeze({ k: 1 }).k);
type Pr = "low" | "medium" | "high";
const c2: Record<Pr, number> = { low: 0, medium: 0, high: 0 };
const ks: Pr[] = ["low", "high", "high", "medium"];
for (const q of ks) c2[q]++;
const kk: Pr = "medium";
c2[kk] += 5;
c2[kk] = c2[kk] * 2;
console.log(c2);
