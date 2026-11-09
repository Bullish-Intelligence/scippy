import { grüße, 日本 } from "./lib";
//       ^^^^^^^ reference scippy npm workspace . `lib.ts`/`grüße`().
//                ^^^^^^ reference scippy npm workspace . `lib.ts`/`日本`.
export const x = grüße("é"); // é 日本 grüße
//               ^^^^^^^ reference scippy npm workspace . `lib.ts`/`grüße`().
export const naïve = 日本;
//           ^^^^^^ definition scippy npm workspace . `use.ts`/`naïve`.
//                    ^^^^^^ reference scippy npm workspace . `lib.ts`/`日本`.
