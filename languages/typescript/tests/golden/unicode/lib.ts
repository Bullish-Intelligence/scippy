  export function grüße(名前: string): string {
//^ definition scippy npm workspace . `lib.ts`/
//  kind Module
//  display_name lib
//                ^^^^^^^ definition scippy npm workspace . `lib.ts`/`grüße`().
//                        kind Function
//                        display_name grüße
//                        ^^^^^^ definition local 0
//                               kind Parameter
//                               display_name 名前
  //              ^^^^^^^ definition scippy npm workspace . `lib.ts`/`grüße`().
  //                      ^^^^^^ definition local 0
    return 名前;
//         ^^^^^^ reference local 0
  //       ^^^^^^ reference local 0
  }
  export const 日本 = 1;
//             ^^^^^^ definition scippy npm workspace . `lib.ts`/`日本`.
//                    kind Variable
//                    display_name 日本
  //           ^^^^^^ definition scippy npm workspace . `lib.ts`/`日本`.
  
