class Circle {
  constructor(private radius: number) {}
  get diameter(): number {
    return this.radius * 2;
  }
  set diameter(d: number) {
    this.radius = d / 2;
  }
  get r(): number {
    return this.radius;
  }
  grow(): void {
    this.diameter = this.diameter + 2;
  }
}
const c = new Circle(2);
console.log(c.diameter, c.r);
c.diameter = 10;
console.log(c.diameter, c.r);
c.diameter += 1;
console.log(c.diameter, c.r);
c.grow();
console.log(c.diameter, c.r);
class Temp {
  private c = 0;
  get f(): number {
    return (this.c * 9) / 5 + 32;
  }
  set f(v: number) {
    this.c = ((v - 32) * 5) / 9;
  }
  get celsius(): number {
    return this.c;
  }
}
const t = new Temp();
t.f = 212;
console.log(t.celsius, t.f);
