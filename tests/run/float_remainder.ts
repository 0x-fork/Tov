const xs = [7, -7, 4, -4, 5.5, -5.5, 0, 1e300, 2 ** 60];
for (const a of xs) for (const b of [2, -2, 3, 0.5, 1e-300, 0]) console.log(a, b, a , Object.is(a , -0));
