import { square } from "@lib/math";
import { greet } from "lib";
export const run = (): string => `${greet("ada")} ${square(7)}`;
