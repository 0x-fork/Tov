// A constructor may assign a field in each branch of an `if` or a `switch`, as TypeScript's
// definite assignment allows.

class Span {
  private start: number;
  private end: number;
  readonly label: string;
  constructor(a?: number | string, b?: number) {
    if (a === undefined) {
      this.start = 0;
      this.end = 0;
    } else if (typeof a === "string") {
      this.start = a.length;
      this.end = a.length * 2;
    } else {
      this.start = a;
      this.end = b ?? a;
    }
    switch (this.end - this.start) {
      case 0:
        this.label = "empty";
        break;
      default:
        this.label = `${this.end - this.start} long`;
    }
  }
  toString(): string {
    return `${this.label} [${this.start}, ${this.end})`;
  }
}

class Parsed {
  readonly value: number;
  constructor(text: string) {
    const n = Number(text);
    if (Number.isNaN(n)) throw new Error(`not a number: ${text}`);
    this.value = n;
  }
}

console.log(`${new Span()}`, `${new Span("abc")}`, `${new Span(2, 7)}`, new Parsed("41").value + 1);
