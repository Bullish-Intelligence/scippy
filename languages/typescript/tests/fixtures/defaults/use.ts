import Widget from "./widget";
//     ^^^^^^ reference scippy npm workspace . `widget.ts`/Widget#
import mk from "./make";
//     ^^ reference scippy npm workspace . `make.ts`/make().
import cfg from "./config";
//     ^^^ reference scippy npm workspace . `config.ts`/config.
import lit from "./literal";
//     ^^^ reference scippy npm workspace . `literal.ts`/default.
export const w = new Widget().render();
//                   ^^^^^^ reference scippy npm workspace . `widget.ts`/Widget#
export const m = mk();
//               ^^ reference scippy npm workspace . `make.ts`/make().
export const c = cfg;
//               ^^^ reference scippy npm workspace . `config.ts`/config.
export const l = lit;
//               ^^^ reference scippy npm workspace . `literal.ts`/default.
