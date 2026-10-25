// localeCompare as ICU's root collation: accents after base letters, case last, ignorables, expansions.
const alpha = [..."aAbBcCeEnNoOsSzZ -_.,'0129éÉèêëÇçñÑöÖøØåÅßæÆœŒłŁđðÐıİžŽšĳǆſŀαΑβяЯё́̈\u0001\t€—“中文한"];
let s = 1;
const rnd = (n: number) => { s = (s * 1103515245 + 12345) % 2147483648; return s % n; };
const word = () => { let w = ""; const n = 1 + rnd(5); for (let i = 0; i < n; i++) w += alpha[rnd(alpha.length)]; return w; };
const out: string[] = [];
let line = "";
for (let i = 0; i < 20000; i++) {
  const a = word(), b = rnd(3) ? word() : a.slice(0, -1) + alpha[rnd(alpha.length)];
  const c = a.localeCompare(b);
  line += c < 0 ? "<" : c > 0 ? ">" : "=";
  if (line.length === 100) { out.push(line); line = ""; }
}
console.log(out.join("\n"));
const names = ["Émile", "zoë", "Adam", "beatrice", "Çelik", "Øystein", "anna", "Bob", "chloé", "David", "eve", "Frank", "Straße", "Strasse", "strasse", "Ærø", "Aero", "co-op", "coop", "co op", "naïve", "naive", "Naive", "résumé", "resume", "Resume", "Łódź", "Lodz", "Ελλάδα", "Москва", "東京", "😀", "á", "á", "10", "9", "_x", "-x"];
console.log([...names].sort((a, b) => a.localeCompare(b)).join(" | "));
console.log("ß".localeCompare("ss"), "ss".localeCompare("ß"), "a\u0001b".localeCompare("ab"), "é".localeCompare("é"), "ae".localeCompare("æ"));
