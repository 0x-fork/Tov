// node:crypto's hashes, HMACs and UUIDs from std/crypto (BoringSSL called directly).
import { createHash, createHmac, randomUUID, hash } from "node:crypto";
console.log(createHash("sha256").update("hello").digest("hex"));
console.log(createHash("md5").update("a").update("b").digest("base64"));
console.log(createHash("sha1").update("").digest("base64url"), createHash("sha512").update("x").digest("hex").length);
console.log(createHmac("sha256", "key").update("data").digest("hex"));
console.log(hash("sha256", "abc"), hash("md5", "abc", "base64"));
const u = randomUUID(); console.log(u.length, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(u));
const h = createHash("sha256"); h.update("x"); h.digest("hex");
try { h.digest("hex"); } catch (e) { console.log((e as Error).message); }
try { createHash("nope"); } catch (e) { console.log((e as Error).message); }
console.log(createHash("sha256").update("héllo 🥟").digest("hex"));
