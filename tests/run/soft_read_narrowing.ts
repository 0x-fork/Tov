function esc(s: string): string { return s.toUpperCase(); }
function code(lang: string | undefined): string {
  const langString = (lang || '').match(/^\S*/)?.[0];
  if (!langString) {
    return "none";
  }
  return esc(langString);
}
console.log(code("js x"), code(undefined));
