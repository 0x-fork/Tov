interface Ext { startInline?: ((src: string) => number | void)[]; renderers?: { [name: string]: (t: string) => string } }
interface Opts { extensions?: null | Ext; pedantic?: boolean }
class Lexer {
  options: Opts;
  constructor(o: Opts) { this.options = o; }
  run(src: string): number {
    let cutSrc = src;
    if (this.options.extensions?.startInline) {
      let startIndex = Infinity;
      const tempSrc = src.slice(1);
      let tempStart;
      this.options.extensions.startInline.forEach((getStartIndex) => {
        tempStart = getStartIndex.call({ lexer: this }, tempSrc);
        if (typeof tempStart === "number" && tempStart >= 0) startIndex = Math.min(startIndex, tempStart);
      });
      if (startIndex < Infinity && startIndex >= 0) cutSrc = src.substring(0, startIndex + 1);
    }
    if (this.options.extensions?.renderers?.[cutSrc]) {
      return this.options.extensions.renderers[cutSrc].call({ parser: this }, cutSrc).length;
    }
    return cutSrc.length;
  }
}
console.log(new Lexer({ extensions: { startInline: [(s) => s.indexOf("c")], renderers: { ab: (t) => t + "!" } } }).run("abcd"));
