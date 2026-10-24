// toISOString and parsing its form, in the runtime: years outside 0-9999, leap days, 24:00.
const ts = [0, 1, -1, 86399999, 951782400000, 1709251199999, -62198755200000, 253402300799999, 253402300800000, -8640000000000000, 8640000000000000, 1e12 + 0.7];
for (const t of ts) { const d = new Date(t); const s = d.toISOString(); console.log(s, new Date(s).getTime() === Math.trunc(t) || new Date(s).getTime(), Date.parse(s)); }
for (const s of ["2024-02-29T12:00:00.000Z", "2023-02-29T12:00:00.000Z", "2024-13-01T00:00:00.000Z", "2024-01-01T24:00:00.000Z", "2024-01-01T24:00:01.000Z", "2024-01-31T23:59:59.999Z", "2024-04-31T00:00:00.000Z", "2024-01-01T00:00:00.000z"]) console.log(s, Date.parse(s));
