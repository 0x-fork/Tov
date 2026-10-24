// A URL's searchParams, changed, change its href and search (the URL standard's update steps).
for (const p of ["/", "/users/42", "/search", "/a/b/../c/./d", "/files/report%202026.pdf"]) {
  const u = new URL(`https://localhost:8080${p}?q=term5&page=3&tag=a&tag=b#frag`);
  console.log(u.pathname, u.host, u.search, u.hash);
  u.searchParams.set("page", "4"); u.searchParams.delete("tag");
  console.log(u.toString(), u.toString().length, u.search);
}
const v = new URL("https://a.b/p?x=1#h"); v.searchParams.delete("x"); console.log(v.href, JSON.stringify(v.search)); v.searchParams.append("y", "a b&c"); console.log(String(v), v.search);
