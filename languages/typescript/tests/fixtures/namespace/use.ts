import * as geo from "./geo";
//          ^^^ reference scippy npm workspace . `geo.ts`/
//                   ^^^^^^^ reference scippy npm workspace . `geo.ts`/
export const v = [geo.area(1), new geo.Circle(), geo.PI];
//                ^^^ reference scippy npm workspace . `geo.ts`/
//                    ^^^^ reference scippy npm workspace . `geo.ts`/area().
//                                     ^^^^^^ reference scippy npm workspace . `geo.ts`/Circle#
//                                                   ^^ reference scippy npm workspace . `geo.ts`/PI.
//                                 ^^^ reference scippy npm workspace . `geo.ts`/
//                                               ^^^ reference scippy npm workspace . `geo.ts`/
