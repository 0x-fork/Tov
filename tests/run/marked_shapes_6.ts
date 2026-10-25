function edit(regex: string) {
  let source = regex;
  const obj = { replace: (a: string, b: string) => { source = source.replace(a, b); return obj; }, getRegex: () => new RegExp(source) };
  return obj;
}
const R = edit("a-b").replace("a", "x").getRegex();
function bad(): number { throw new Error("boom"); }
console.log(R.source);
const V: number = JSON.parse("[1]").length;
console.log(V);
