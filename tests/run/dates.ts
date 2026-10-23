// Dates, as JavaScript's: milliseconds since 1970 UTC, local fields in the system's time zone.
// (Only what reads the same in every time zone is printed.)

const a = new Date("2019-01-16");
const b = new Date("2019-01-01T08:30:15.250Z");
console.log(a, [a, b], { when: b });
console.log(a.getTime(), a.toISOString(), a.getUTCDay(), a.getUTCFullYear(), a.getUTCMonth(), a.getUTCDate());
console.log(b.getUTCHours(), b.getUTCMinutes(), b.getUTCSeconds(), b.getUTCMilliseconds(), b.toUTCString());
console.log(JSON.stringify({ a }), a.toJSON(), new Date(NaN).toJSON(), String(new Date("nope")));
console.log(new Date(Date.UTC(2020, 1, 29, 13, 5, 7, 42)).toISOString(), Date.UTC(99, 0), Date.parse("2020-02-30"), Date.parse("2020-13-01"));
console.log(new Date(0).toISOString(), new Date(-1).toISOString(), new Date(8.64e15).toISOString(), new Date(8.64e15 + 1).getTime());
console.log(new Date("+275760-09-13T00:00:00.000Z").getTime(), new Date("-000001-01-01T00:00:00Z").toISOString(), new Date("2019-01-16T08:30:00+05:30").toISOString());

// comparing and subtracting: by their times
console.log(a > b, a < b, a >= b, b <= a, a - b, +a, -b, a === new Date(a), a.getTime() === new Date(a).getTime());
const xs = [a, b, new Date("2022-05-20")];
xs.sort((x, y) => x.getTime() - y.getTime());
console.log(xs.map((d) => d.toISOString().slice(0, 10)));
function latest<T>(p: T, q: T): T {
  return p > q ? p : q;
}
console.log(latest(a, b).toISOString());

// local fields: a date made from them reads them back
const c = new Date(2024, 0, 31, 10, 20, 30, 400);
console.log(c.getFullYear(), c.getMonth(), c.getDate(), c.getHours(), c.getMinutes(), c.getSeconds(), c.getMilliseconds());
c.setMonth(1);
console.log(c.getMonth(), c.getDate());
c.setDate(0);
console.log(c.getMonth(), c.getDate(), c.getDay());
c.setHours(25, 61, 61, 1001);
console.log(c.getDate(), c.getHours(), c.getMinutes(), c.getSeconds(), c.getMilliseconds());
c.setUTCFullYear(1999, 11, 31);
console.log(c.getUTCFullYear(), c.getUTCMonth(), c.getUTCDate(), c.getUTCDay());
console.log(new Date(99, 0).getFullYear(), new Date(2020, 1, 29, 24).getDate(), new Date(2020, 0).getTime() === new Date("2020-01-01T00:00").getTime());
console.log(new Date(a) instanceof Date, typeof a, Date.now() > 1.6e12, new Date().getTime() <= Date.now());
