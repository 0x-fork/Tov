interface Link { type: "link"; raw: string; href: string; text: string }
interface Image { type: "image"; raw: string; href: string; text: string }
function make(isImage: boolean, raw: string): Link | Image | undefined {
  return { type: isImage ? "image" : "link", raw, href: "/" + raw, text: raw.toUpperCase() };
}
for (const t of [make(true, "a"), make(false, "b")]) console.log(t?.type, t?.href, t?.text);
