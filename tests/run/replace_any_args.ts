const rules = { findPipe: /\|/g, ref: /&(?:#(\d+)|#x([0-9a-f]+)|amp);/gi, word: /(\w)(\w*)/g };
const row = "a|b\\|c|d".replace(rules.findPipe, (match, offset, str) => {
  let escaped = false;
  let curr = offset;
  while (--curr >= 0 && str[curr] === "\\") escaped = !escaped;
  return escaped ? "|" : " |";
});
console.log(row);
const t = "x &#65; &#x42; &amp; y".replace(rules.ref, (ref: string, dec?: string, hex?: string) => {
  if (dec) return String.fromCharCode(parseInt(dec, 10));
  if (hex) return String.fromCharCode(parseInt(hex, 16));
  return "&";
});
console.log(t);
console.log("hello world".replace(rules.word, (m, a, b) => a.toUpperCase() + b));
