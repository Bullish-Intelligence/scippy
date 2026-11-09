import { alpha, gamma, core } from "./index";
//       ^^^^^ reference scippy npm workspace . `core/a.ts`/alpha.
//              ^^^^^ reference scippy npm workspace . `core/a.ts`/beta().
//                     ^^^^ reference scippy npm workspace . `core/index.ts`/
export const x = [alpha, gamma(), core.beta()];
//                ^^^^^ reference scippy npm workspace . `core/a.ts`/alpha.
//                       ^^^^^ reference scippy npm workspace . `core/a.ts`/beta().
//                                ^^^^ reference scippy npm workspace . `core/index.ts`/
//                                     ^^^^ reference scippy npm workspace . `core/a.ts`/beta().
