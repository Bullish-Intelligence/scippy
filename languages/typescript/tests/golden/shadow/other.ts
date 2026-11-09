  export function process(x: string): string {
//^ definition scippy npm workspace . `other.ts`/
//  kind Module
//  display_name other
//                ^^^^^^^ definition scippy npm workspace . `other.ts`/process().
//                        kind Function
//                        display_name process
//                        ^ definition local 0
//                          kind Parameter
//                          display_name x
  //              ^^^^^^^ definition scippy npm workspace . `other.ts`/process().
    return x;
//         ^ reference local 0
  }
  export const second = process('a');
//             ^^^^^^ definition scippy npm workspace . `other.ts`/second.
//                    kind Variable
//                    display_name second
//                      ^^^^^^^ reference scippy npm workspace . `other.ts`/process().
  //                    ^^^^^^^ reference scippy npm workspace . `other.ts`/process().
  
