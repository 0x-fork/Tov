// `n = g.next().value` of a `Generator<T>`: TypeScript leaves the result `any`, so a value is
// read where one must be (checked to be there).

function* naturals(): Generator<number> {
  let n = 0;
  while (true) yield n++;
}
const g = naturals();
let last = 0;
for (let i = 0; i < 5; i++) last = g.next().value;
const r = g.next();
console.log(last, r.value, r.done);
function* two(): Generator<string> {
  yield "a";
  yield "b";
}
const t = two();
const first: string = t.next().value;
console.log(first, t.next().value, t.next().done);
