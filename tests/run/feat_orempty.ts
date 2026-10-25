function words(s: string): string[] {
  return s.match(/\w+/g) || [];
}
function firstOr(xs: number[] | undefined): number[] {
  return xs || [];
}
class Box<T, V extends Iterable<T> = Iterable<T>> {
  items(v: V): T[] { return Array.from(v); }
  join(xs: T[]): string { return (xs as unknown as string[]).join("-"); }
}
const b = new Box<string, string>();
console.log(JSON.stringify(words("a bb  ccc")), JSON.stringify(words("  ")), JSON.stringify(firstOr(undefined)), JSON.stringify(firstOr([1, 2])), JSON.stringify(b.items("xyz")), b.join(["p", "q"]));
