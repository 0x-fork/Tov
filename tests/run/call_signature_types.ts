// A record type that is only a call signature, `{ (x: T): R }`, is that function type.

type Search = { (array: number[], target: number): number | null };
type Searches = { (array: number[], target: number): number | null }[];

const linear: Search = (array, target) => {
  const i = array.indexOf(target);
  return i < 0 ? null : i;
};
const fromEnd = (array: number[], target: number): number | null => {
  const i = array.lastIndexOf(target);
  return i < 0 ? null : i;
};

const all: Searches = [linear, fromEnd];
for (const f of all) {
  console.log(f([1, 2, 3, 2], 2), f([1], 5));
}
