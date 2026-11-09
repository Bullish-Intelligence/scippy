  import { alpha, gamma, core } from "./index";
//^ definition scippy npm workspace . `use.ts`/
//  kind Module
//  display_name use
//         ^^^^^ reference scippy npm workspace . `core/a.ts`/alpha.
//                ^^^^^ reference scippy npm workspace . `core/a.ts`/beta().
//                       ^^^^ reference scippy npm workspace . `core/index.ts`/
//                                   ^^^^^^^^^ reference scippy npm workspace . `index.ts`/
  //       ^^^^^ reference scippy npm workspace . `core/a.ts`/alpha.
  //              ^^^^^ reference scippy npm workspace . `core/a.ts`/beta().
  //                     ^^^^ reference scippy npm workspace . `core/index.ts`/
  export const x = [alpha, gamma(), core.beta()];
//             ^ definition scippy npm workspace . `use.ts`/x.
//               kind Variable
//               display_name x
//                  ^^^^^ reference scippy npm workspace . `core/a.ts`/alpha.
//                         ^^^^^ reference scippy npm workspace . `core/a.ts`/beta().
//                                  ^^^^ reference scippy npm workspace . `core/index.ts`/
//                                       ^^^^ reference scippy npm workspace . `core/a.ts`/beta().
  //                ^^^^^ reference scippy npm workspace . `core/a.ts`/alpha.
  //                       ^^^^^ reference scippy npm workspace . `core/a.ts`/beta().
  //                                ^^^^ reference scippy npm workspace . `core/index.ts`/
  //                                     ^^^^ reference scippy npm workspace . `core/a.ts`/beta().
  
