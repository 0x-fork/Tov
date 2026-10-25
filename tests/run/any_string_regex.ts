const a: any = "[x] foo";
console.log(a.replace(/^\[[ xX]\] +/, ""), a.replace("foo", "bar"), a.toUpperCase(), a.startsWith("["), a.slice(1), a.split(" ").length, a.trim(), a.indexOf("f"));
