// Chai's `expect` assertions (as Mocha tests use them) are the built-in matchers.
import { expect } from "chai";

class Box {
  constructor(public n: number) {}
}

describe("chai", function () {
  it("compares", function () {
    expect(1 + 1).to.equal(2);
    expect([1, 2]).to.deep.equal([1, 2]);
    expect({ a: 1 }).to.eql({ a: 1 });
    expect("abc").to.not.equal("abd");
    expect(3).to.be.above(2);
    expect(3).to.be.at.most(3);
    expect(0.1 + 0.2).to.be.closeTo(0.3, 1e-9);
  });
  it("checks kinds", function () {
    expect(true).to.be.true;
    expect(null).to.be.null;
    expect(undefined).to.be.undefined;
    expect([1, 2, 3]).to.have.lengthOf(3);
    expect([1, 2, 3]).to.include(2);
    expect("hello").to.contain("ell");
    expect([]).to.be.empty;
    expect(new Box(1)).to.be.instanceOf(Box);
    expect([1]).to.be.an("array");
    expect("x").to.be.a("string");
    expect(() => JSON.parse("{")).to.throw();
  });
  it("fails", function () {
    expect(2).to.equal(3);
  });
});
