  import { Button } from "./Button";
//^ definition scippy npm workspace . `app.tsx`/
//  kind Module
//  display_name app
//         ^^^^^^ reference scippy npm workspace . `Button.tsx`/Button().
//                       ^^^^^^^^^^ reference scippy npm workspace . `Button.tsx`/
  //       ^^^^^^ reference scippy npm workspace . `Button.tsx`/Button().
  export const app = <Button label="a" />;
//             ^^^ definition scippy npm workspace . `app.tsx`/app.
//                 kind Variable
//                 display_name app
//                    ^^^^^^ reference scippy npm workspace . `Button.tsx`/Button().
  //                  ^^^^^^ reference scippy npm workspace . `Button.tsx`/Button().
  
