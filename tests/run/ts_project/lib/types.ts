export interface Sale { region: string; product: string; units: number; price: number; date: string }
export type Summary = { region: string; revenue: number; units: number; top: string };
export enum Level { Low = "low", Mid = "mid", High = "high" }
