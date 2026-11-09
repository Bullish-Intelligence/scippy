import type { Shape, Id } from "./types";
//            ^^^^^ reference scippy npm workspace . `types.ts`/Shape#
//                   ^^ reference scippy npm workspace . `types.ts`/Id#
import { type Impl } from "./types";
//            ^^^^ reference scippy npm workspace . `types.ts`/Impl#
export function describe(s: Shape, id: Id, i?: Impl): number {
//                          ^^^^^ reference scippy npm workspace . `types.ts`/Shape#
//                                     ^^ reference scippy npm workspace . `types.ts`/Id#
//                                             ^^^^ reference scippy npm workspace . `types.ts`/Impl#
  return s.area();
//         ^^^^ reference scippy npm workspace . `types.ts`/Shape#area().
}
