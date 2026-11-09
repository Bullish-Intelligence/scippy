import { a } from "./a";
//       ^ reference scippy npm workspace . `a.ts`/a().
export function b(): number {
//              ^ definition scippy npm workspace . `b.ts`/b().
  return a();
}
