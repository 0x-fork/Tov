import { register } from "./registry.ts";
register("alpha");
register("beta");
for (let i = 0; i < 2; i++) register(`p${i}`);
console.log("plugins loaded");
export const count = 4;
