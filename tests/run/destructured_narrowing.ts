// A destructured discriminant narrows its siblings, as TypeScript's do: `const { done, value } =
// it.next(); if (done) break;` makes `value` what the generator yields.

function* count(): Generator<number> {
  yield 1;
  yield 2;
}
const it = count();
let total = 0;
while (true) {
  const { done, value } = it.next();
  if (done) break;
  total += value;
}

type Reply = { ok: true; body: string } | { ok: false; body: number };
function describe(r: Reply): string {
  const { ok, body } = r;
  if (ok === false) return `error ${body + 1}`;
  return body.toUpperCase();
}
const r = it.next();
console.log(total, describe({ ok: true, body: "fine" }), describe({ ok: false, body: 403 }), r.done, r.value);
