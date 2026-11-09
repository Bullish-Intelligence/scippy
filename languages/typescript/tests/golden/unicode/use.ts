  import { grüße, 日本 } from "./lib";
//^ definition scippy npm workspace . `use.ts`/
//  kind Module
//  display_name use
//         ^^^^^^^ reference scippy npm workspace . `lib.ts`/`grüße`().
//                  ^^^^^^ reference scippy npm workspace . `lib.ts`/`日本`.
//                                ^^^^^^^ reference scippy npm workspace . `lib.ts`/
  //       ^^^^^^^ reference scippy npm workspace . `lib.ts`/`grüße`().
  //                ^^^^^^ reference scippy npm workspace . `lib.ts`/`日本`.
  export const x = grüße("é"); // é 日本 grüße
//             ^ definition scippy npm workspace . `use.ts`/x.
//               kind Variable
//               display_name x
//                 ^^^^^^^ reference scippy npm workspace . `lib.ts`/`grüße`().
  //               ^^^^^^^ reference scippy npm workspace . `lib.ts`/`grüße`().
  export const naïve = 日本;
//             ^^^^^^ definition scippy npm workspace . `use.ts`/`naïve`.
//                    kind Variable
//                    display_name naïve
//                      ^^^^^^ reference scippy npm workspace . `lib.ts`/`日本`.
  //           ^^^^^^ definition scippy npm workspace . `use.ts`/`naïve`.
  //                    ^^^^^^ reference scippy npm workspace . `lib.ts`/`日本`.
  
