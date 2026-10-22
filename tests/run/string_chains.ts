const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const out: string[] = [];
for (let i = 0; i < 70; i += 3) out.push(chars[i % 64] + chars[(i + 1) % 64] + "-" + chars[(i * 7) % 64] + String(i));
console.log(out.join("|"));
const a = "x", b = 1;
console.log(a + b + "y" + a, 1 + 2 + "z" + 3 + 4, "é" + a + "ü" + "€");
