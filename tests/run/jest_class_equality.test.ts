// `toEqual` compares class instances with objects of the same fields, as Jest does.

class Entry<K, V> {
  constructor(public key: K, public value: V) {}
}

class Point {
  constructor(public x: number, public y: number) {}
  norm(): number {
    return Math.abs(this.x) + Math.abs(this.y);
  }
}

class Empty {}

test("instances equal objects", () => {
  const entries = [new Entry("a", 1), new Entry("b", 2)];
  expect(entries).toEqual([{ key: "a", value: 1 }, { key: "b", value: 2 }]);
  expect(new Point(1, 2)).toEqual({ x: 1, y: 2 });
  expect([new Empty()]).toEqual([{}]);
  expect(new Point(1, 2)).not.toEqual({ x: 1, y: 3 });
});

test("a field differs", () => {
  expect([new Point(3, 4)]).toEqual([{ x: 3, y: 5 }]);
});
