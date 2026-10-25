function edit(regex: string, opt = "") {
  let source = regex;
  const obj = {
    replace: (name: string, val: string) => {
      source = source.replace(name, val);
      return obj;
    },
    getRegex: () => new RegExp(source, opt),
  };
  return obj;
}
const re = edit("a-b-c").replace("a", "x").replace("c", "z").getRegex();
console.log(re.source, re.test("x-b-z"));
