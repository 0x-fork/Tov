// Object literals with computed and number keys: a Record of string literal keys (an enum's) is a
// record, others are maps, number-keyed too; Object.keys gives strings.
enum Color { Red = "red", Green = "green" }
const hex: Record<Color, string> = { [Color.Red]: "#f00", [Color.Green]: "#0f0" }
console.log(hex[Color.Green], hex.red)
const hex: Record<Color, string> = { [Color.Red]: "#f00", [Color.Green]: "#0f0" }
interface User { id: number; name: string }
const db: Record<number, User> = { 1: { id: 1, name: "Ann" }, 2: { id: 2, name: "Ben" } }
const k = "dyn"
const m: Record<string, number> = { [k]: 1, plain: 2, ["a" + "b"]: 3 }
console.log(hex[Color.Green], db[2]!.name, m["dyn"], m.plain, m["ab"], Object.keys(m).join(","))
