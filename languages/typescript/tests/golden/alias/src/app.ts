  import { process as run } from "./services.js";
//^ definition scippy npm workspace . `src/app.ts`/
//  kind Module
//  display_name app
//         ^^^^^^^ reference scippy npm workspace . `src/services.ts`/process().
//                    ^^^ reference scippy npm workspace . `src/services.ts`/process().
//                               ^^^^^^^^^^^^^^^ reference scippy npm workspace . `src/services.ts`/
  //       ^^^^^^^ reference scippy npm workspace . `src/services.ts`/process().
  //                  ^^^ reference scippy npm workspace . `src/services.ts`/process().
  //                             ^^^^^^^^^^^^^^^ reference scippy npm workspace . `src/services.ts`/
  const result = run("hello");
//      ^^^^^^ definition scippy npm workspace . `src/app.ts`/result.
//             kind Variable
//             display_name result
//               ^^^ reference scippy npm workspace . `src/services.ts`/process().
  //    ^^^^^^ definition scippy npm workspace . `src/app.ts`/result.
  //             ^^^ reference scippy npm workspace . `src/services.ts`/process().
  
