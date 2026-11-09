import { b } from "./b";
//       ^ reference scippy npm workspace . `b.ts`/b().
export function a(): number {
//              ^ definition scippy npm workspace . `a.ts`/a().
  return b();
}
