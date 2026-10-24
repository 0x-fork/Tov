// s.replace(/re/g, (m) => ...) with a callback that can't throw: compiled to a loop over the matches.
const map: Record<string, string> = { "&": "&amp;", "<": "&lt;" };
const esc = (s: string) => s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c] ?? c);
console.log(esc(`<a href="x">Tom & 'Jerry'</a>`), esc("plain"), esc(""), esc("é<ü>🥟&"));
let n = 0;
const prefix = "#";
console.log("a1b22c333".replace(/\d+/g, (d) => prefix + d.length), "aXbX".replace(/x/i, (m) => m.toLowerCase()));
console.log("x-y".replace(/(?:)/g, () => "|"), "🥟🥟".replace(/(?:)/gu, () => "."), "abc".replace(/b/g, (m) => map[m] ?? m.toUpperCase()));
const re = /o/g; re.lastIndex = 3; console.log("foo boo".replace(re, (m) => "0"), re.lastIndex);
function wrap(s: string, tag: string) { return s.replace(/\w+/g, (w) => `<${tag}>${w}</${tag}>`); }
console.log(wrap("hello world", "b"));
