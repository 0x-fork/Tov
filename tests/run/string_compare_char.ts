// Comparing a string with a one-character ASCII literal (`c >= "0"`): its first byte, then its length.
const xs = ["", "0", "5", "9", "a", "Z", "09", "/", ":", "é", "😀", " ", "00", "0a"];
const out: string[] = [];
for (const x of xs) out.push(`${JSON.stringify(x)}:${x < "0" ? 1 : 0}${x <= "0" ? 1 : 0}${x > "9" ? 1 : 0}${x >= "9" ? 1 : 0}${"5" < x ? 1 : 0}${"5" >= x ? 1 : 0}${x >= "a" && x <= "z" ? 1 : 0}`);
console.log(out.join(" "));
