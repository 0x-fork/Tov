class Hooks {
  block?: boolean;
  constructor(b?: boolean) { this.block = b; }
  pick(block = this.block): string { return block ? "block" : "inline"; }
}
console.log(new Hooks(true).pick(), new Hooks().pick(), new Hooks(true).pick(false));
