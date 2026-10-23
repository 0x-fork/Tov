export const names: string[] = [];
export function register(name: string): void {
  names.push(name);
}
console.log("registry loaded");
