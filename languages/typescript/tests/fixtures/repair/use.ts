import type { Reader } from "./types";
//            ^^^^^^ reference scippy npm workspace . `types.ts`/Reader#
import { call } from "./types";
//       ^^^^ reference scippy npm workspace . `types.ts`/call.
export const r = call(1);
//               ^^^^ reference scippy npm workspace . `types.ts`/call.
export type R = Reader<number>;
//              ^^^^^^ reference scippy npm workspace . `types.ts`/Reader#
