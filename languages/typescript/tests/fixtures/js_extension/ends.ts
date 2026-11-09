import { limits } from "./lib/constants";
//       ^^^^^^ reference scippy npm workspace . `lib/constants.ts`/limits.
import { jsx } from "./lib/widgets";
//       ^^^ reference scippy npm workspace . `lib/widgets.ts`/jsx.
export const both = [limits, jsx];
//                   ^^^^^^ reference scippy npm workspace . `lib/constants.ts`/limits.
//                           ^^^ reference scippy npm workspace . `lib/widgets.ts`/jsx.
