import { process as run } from "./services.js";
//       ^^^^^^^ reference scippy npm workspace . `src/services.ts`/process().
//                  ^^^ reference scippy npm workspace . `src/services.ts`/process().
//                             ^^^^^^^^^^^^^^^ reference scippy npm workspace . `src/services.ts`/
const result = run("hello");
//    ^^^^^^ definition scippy npm workspace . `src/app.ts`/result.
//             ^^^ reference scippy npm workspace . `src/services.ts`/process().
