import { feather } from "@lib/f";
//       ^^^^^^^ reference scippy npm @acme/app 1.2.3 `src/lib/f.ts`/feather().
import { cfg } from "@cfg";
//       ^^^ reference scippy npm @acme/app 1.2.3 `src/shared/cfg.ts`/cfg.
import { gadget } from "shared/g";
//       ^^^^^^ reference scippy npm @acme/app 1.2.3 `src/shared/g.ts`/gadget.
import { feather as self } from "@acme/app/src/lib/f";
//       ^^^^^^^ reference scippy npm @acme/app 1.2.3 `src/lib/f.ts`/feather().
export const all = [feather(), cfg, gadget, self()];
//                  ^^^^^^^ reference scippy npm @acme/app 1.2.3 `src/lib/f.ts`/feather().
//                             ^^^ reference scippy npm @acme/app 1.2.3 `src/shared/cfg.ts`/cfg.
//                                  ^^^^^^ reference scippy npm @acme/app 1.2.3 `src/shared/g.ts`/gadget.
//                                          ^^^^ reference scippy npm @acme/app 1.2.3 `src/lib/f.ts`/feather().
