export namespace Tokens {
  export interface Space { type: "space"; raw: string }
  export interface Text { type: "text"; raw: string; text: string }
  export function make(raw: string): Text { return { type: "text", raw, text: raw.trim() }; }
  export const VERSION = "1.0";
}
export type Token = Tokens.Space | Tokens.Text;
