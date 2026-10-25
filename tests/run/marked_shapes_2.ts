const map: { [k: string]: string } = { "&": "&amp;", "<": "&lt;" };
const rep = (ch: string) => map[ch];
console.log("a<b&c".replace(/[&<]/g, rep));
