// Map.set and Set.add give the map or set back, so calls chain, as in JavaScript.
const s = new Set<number>()
s.add(10).add(2).add(10)
const m = new Map<string, number>().set("a", 1).set("b", 2)
const m2 = m.set("c", 3)
console.log(s, m, m2 === m, new Set<string>().add("x").size)
