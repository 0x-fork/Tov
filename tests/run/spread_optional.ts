// Spreads: an optional field of a later one keeps an earlier value where it is absent; object
// literals spread as their own fields.
interface P { sku: string; name: string; qty: number }
type U = Partial<P>
const a: P = { sku: "s1", name: "bolt", qty: 3 }
const ch: U = { qty: 7 }
const b: P = { ...a, ...ch }
const c = { ...a, ...{ name: "nut" } }
console.log(b, c)
