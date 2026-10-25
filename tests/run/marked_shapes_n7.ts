function edit(regex: string, opt = "") {
  let source = regex;
  const obj = {
    replace: (name: string, val: string) => { source = source.replace(name, val); return obj; },
    getRegex: () => new RegExp(source, opt),
  };
  return obj;
}
const rules = { delim: edit("a(b)").replace("b", "c").getRegex() };
function f(s: string): number {
  const endReg = rules.delim;
  endReg.lastIndex = 0;
  const m = endReg.exec(s);
  let len = 0;
  if (m) len = [...m[0]].length;
  return Math.min(len, 5);
}
console.log(f("xacx"));
