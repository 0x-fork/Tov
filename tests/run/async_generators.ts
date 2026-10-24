// async function* and for await: next() promises, awaits inside, early and late next()s.
const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
async function* count(n: number): AsyncGenerator<number> {
  for (let i = 0; i < n; i++) {
    await sleep(1);
    yield i * 10;
  }
}
async function* words() {
  yield "a";
  await Promise.resolve(0);
  yield "b";
}
async function main() {
  const g = count(3);
  console.log(await g.next(), await g.next());
  let total = 0;
  for await (const x of count(4)) total += x;
  console.log(total);
  const out: string[] = [];
  for await (const w of words()) out.push(w);
  console.log(out.join(","));
  const r = await g.next();
  console.log(r.done, r.value, (await g.next()).done);
}
main();
