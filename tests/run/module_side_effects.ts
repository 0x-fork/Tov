// An imported module's top-level statements run once, before the importer's, each module after
// those it imports, as JavaScript runs them.
import { names } from "./module_side_effects/registry.ts";
import { count } from "./module_side_effects/plugins.ts";

console.log("main", names, count);
