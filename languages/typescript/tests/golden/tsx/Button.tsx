  import { shout } from "./text";
//^ definition scippy npm workspace . `Button.tsx`/
//  kind Module
//  display_name Button
//         ^^^^^ reference scippy npm workspace . `text.ts`/shout().
//                      ^^^^^^^^ reference scippy npm workspace . `text.ts`/
  //       ^^^^^ reference scippy npm workspace . `text.ts`/shout().
  const button = 1;
//      ^^^^^^ definition scippy npm workspace . `Button.tsx`/button.
//             kind Variable
//             display_name button
  //    ^^^^^^ definition scippy npm workspace . `Button.tsx`/button.
  export function Button(props: { label: string }) {
//                ^^^^^^ definition scippy npm workspace . `Button.tsx`/Button().
//                       kind Function
//                       display_name Button
//                       ^^^^^ definition local 0
//                             kind Parameter
//                             display_name props
//                                ^^^^^ definition local 1
//                                      kind Field
//                                      display_name label
  //              ^^^^^^ definition scippy npm workspace . `Button.tsx`/Button().
    return <button>{shout(props.label)}</button>;
//                  ^^^^^ reference scippy npm workspace . `text.ts`/shout().
//                        ^^^^^ reference local 0
//                              ^^^^^ reference local 1
  //                ^^^^^ reference scippy npm workspace . `text.ts`/shout().
  }
  
