class Hooks<P = string> {
  options: Opts<P>;
  constructor(o?: Opts<P>) { this.options = o || defaults(); }
  preprocess(md: string): string { return md; }
  postprocess(html: P): P { return html; }
}
type HooksApi<P = string> = Omit<Hooks<P>, "constructor" | "options">;
type HooksObject<P = string> = { [K in keyof HooksApi<P>]?: (...args: Parameters<HooksApi<P>[K]>) => ReturnType<HooksApi<P>[K]> };
interface Ext<P = string> { gfm?: boolean; hooks?: HooksObject<P> | null; silent?: boolean }
interface Opts<P = string> extends Omit<Ext<P>, "hooks"> { hooks?: Hooks<P> | null }
function defaults(): Opts<any> { return { gfm: true, hooks: null, silent: false }; }
const o: Opts = defaults();
const h = new Hooks(o);
console.log(h.preprocess("x"), o.gfm, o.silent);
const ext: Ext = { hooks: { preprocess: (s) => s + "!" } };
console.log(typeof ext.hooks?.preprocess);
