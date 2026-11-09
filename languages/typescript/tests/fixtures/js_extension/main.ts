import { answer } from "./lib/util.js";
//       ^^^^^^ reference scippy npm workspace . `lib/util.ts`/answer.
//                     ^^^^^^^^^^^^^^^ reference scippy npm workspace . `lib/util.ts`/
import { answer as again } from "./lib/util";
//       ^^^^^^ reference scippy npm workspace . `lib/util.ts`/answer.
//                              ^^^^^^^^^^^^ reference scippy npm workspace . `lib/util.ts`/
import { helper } from "./lib";
//       ^^^^^^ reference scippy npm workspace . `lib/index.ts`/helper().
//                     ^^^^^^^ reference scippy npm workspace . `lib/index.ts`/
import { helper as help2 } from "./lib/index.js";
//       ^^^^^^ reference scippy npm workspace . `lib/index.ts`/helper().
//                              ^^^^^^^^^^^^^^^^ reference scippy npm workspace . `lib/index.ts`/
export const total = answer + again + helper() + help2();
//                   ^^^^^^ reference scippy npm workspace . `lib/util.ts`/answer.
//                            ^^^^^ reference scippy npm workspace . `lib/util.ts`/answer.
//                                    ^^^^^^ reference scippy npm workspace . `lib/index.ts`/helper().
//                                               ^^^^^ reference scippy npm workspace . `lib/index.ts`/helper().
