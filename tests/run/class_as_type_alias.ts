// A class instance goes where an object type alias with its fields is expected, as TypeScript's
// types are structural; values of the alias alone stay plain records.
type Point = { x: number; y: number }
type Named = { name: string; area(): number }

class Pt {
  x: number
  y: number
  constructor(x: number, y: number) {
    this.x = x
    this.y = y
  }
}

class Square {
  name = "square"
  side: number
  constructor(side: number) {
    this.side = side
  }
  area(): number {
    return this.side * this.side
  }
}

function norm1(p: Point): number {
  return Math.abs(p.x) + Math.abs(p.y)
}

const pts: Point[] = [{ x: 1, y: 2 }, new Pt(3.5, -4)]
console.log(pts.map(norm1), norm1(new Pt(1, 1)))
const shapes: Named[] = [new Square(3), { name: "unit", area: () => 1 }]
for (const s of shapes) console.log(s.name, s.area())
