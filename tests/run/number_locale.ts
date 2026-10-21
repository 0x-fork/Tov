console.log((12345.6789).toLocaleString("en-US"), (1234567).toLocaleString(), (0.5).toLocaleString());
console.log((-0).toLocaleString(), (-0.0001).toLocaleString(), (1e21).toLocaleString(), (NaN).toLocaleString(), (-Infinity).toLocaleString());
console.log((1.0005).toLocaleString(), (999.9996).toLocaleString(), (-1234.5).toLocaleString("en"), (100).toLocaleString());
console.log((5).toLocaleString("en-US", { minimumFractionDigits: 2 }), (1234.5).toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 }));
console.log((3.14159).toLocaleString(undefined, { maximumFractionDigits: 0 }), (2.5).toLocaleString(undefined, { maximumFractionDigits: 0 }), (0.125).toLocaleString(undefined, { maximumFractionDigits: 2 }));
let total = 0;
for (let i = 0; i < 10; i++) total += i * 1234.25;
console.log(total.toLocaleString(), (123456789012).toLocaleString(), (-5).toLocaleString(undefined, { minimumFractionDigits: 1 }));
console.log((2 ** 70).toLocaleString(), (1e300).toLocaleString().length, (9.9995).toLocaleString(), (0.0005).toLocaleString(), (-0.0005).toLocaleString(), (99.5).toLocaleString(undefined,{maximumFractionDigits:0}), (1e-7).toLocaleString(), (123.456).toLocaleString(undefined,{minimumFractionDigits:5}))
console.log(2 ** 70, 2 ** -1, 2 ** 10, (-2) ** 3, 10 ** 15, 10 ** 16, 3 ** 40, (2 ** 70).toLocaleString()); const p = 2 ** 0.5; console.log(p)
console.log(Math.ceil(-0.5), Math.round(-0.4), Math.round(-0.5), Math.round(-0.6), Math.trunc(-0.9), Math.ceil(-1.5), Math.floor(-0.5), Math.round(2.5)); const x = -0.4; console.log(Math.round(x) + 1)
