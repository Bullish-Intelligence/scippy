  export function process(x: number): number {
//^ definition scippy npm workspace . `outer.ts`/
//  kind Module
//  display_name outer
//                ^^^^^^^ definition scippy npm workspace . `outer.ts`/process().
//                        kind Function
//                        display_name process
//                        ^ definition local 0
//                          kind Parameter
//                          display_name x
  //              ^^^^^^^ definition scippy npm workspace . `outer.ts`/process().
    return x + 1;
//         ^ reference local 0
  }
  export const first = process(2);
//             ^^^^^ definition scippy npm workspace . `outer.ts`/first.
//                   kind Variable
//                   display_name first
//                     ^^^^^^^ reference scippy npm workspace . `outer.ts`/process().
  //                   ^^^^^^^ reference scippy npm workspace . `outer.ts`/process().
  export function shadowed(): number {
//                ^^^^^^^^ definition scippy npm workspace . `outer.ts`/shadowed().
//                         kind Function
//                         display_name shadowed
    const process = 5;
//        ^^^^^^^ definition local 1
//                kind Variable
//                display_name process
  //      ^^^^^^^ definition local 1
    return process;
//         ^^^^^^^ reference local 1
  //       ^^^^^^^ reference local 1
  }
  
