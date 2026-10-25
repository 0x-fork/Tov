interface Opts { a?: number; b?: string }
class Base {
  run(v: string, o: Opts): string { return "base " + v + (o.a ?? 0); }
}
class Sub extends Base {
  run(v: string, o: Opts | { c: boolean } = {}): string { return "sub " + v + JSON.stringify(o); }
}
const xs: Base[] = [new Base(), new Sub()];
for (const x of xs) console.log(x.run("v", { a: 2 }));
console.log(new Sub().run("w"));
