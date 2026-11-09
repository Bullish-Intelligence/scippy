  export * from "./a";
//^ definition scippy npm workspace . `core/index.ts`/
//  kind Module
//  display_name core
//              ^^^^^ reference scippy npm workspace . `core/a.ts`/
  export { beta as gamma } from "./a";
//         ^^^^ reference scippy npm workspace . `core/a.ts`/beta().
//                 ^^^^^ reference scippy npm workspace . `core/a.ts`/beta().
//                              ^^^^^ reference scippy npm workspace . `core/a.ts`/
  //       ^^^^ reference scippy npm workspace . `core/a.ts`/beta().
  
