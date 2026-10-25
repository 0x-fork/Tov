class Hooks { mask(s: string): string { return s.toUpperCase(); } }
interface Opts { hooks?: Hooks | null }
const o: Opts = { hooks: new Hooks() };
const none: Opts = {};
console.log(o.hooks?.mask?.call({ lexer: 1 }, "abc") ?? "x", none.hooks?.mask?.call({}, "abc") ?? "fallback", new Hooks().mask.apply(null, ["q"]));
