  import Widget from "./widget";
//^ definition scippy npm workspace . `use.ts`/
//  kind Module
//  display_name use
//       ^^^^^^ reference scippy npm workspace . `widget.ts`/Widget#
//                   ^^^^^^^^^^ reference scippy npm workspace . `widget.ts`/
  //     ^^^^^^ reference scippy npm workspace . `widget.ts`/Widget#
  import mk from "./make";
//       ^^ reference scippy npm workspace . `make.ts`/make().
//               ^^^^^^^^ reference scippy npm workspace . `make.ts`/
  //     ^^ reference scippy npm workspace . `make.ts`/make().
  import cfg from "./config";
//       ^^^ reference scippy npm workspace . `config.ts`/config.
//                ^^^^^^^^^^ reference scippy npm workspace . `config.ts`/
  //     ^^^ reference scippy npm workspace . `config.ts`/config.
  import lit from "./literal";
//       ^^^ reference scippy npm workspace . `literal.ts`/default.
//                ^^^^^^^^^^^ reference scippy npm workspace . `literal.ts`/
  //     ^^^ reference scippy npm workspace . `literal.ts`/default.
  export const w = new Widget().render();
//             ^ definition scippy npm workspace . `use.ts`/w.
//               kind Variable
//               display_name w
//                     ^^^^^^ reference scippy npm workspace . `widget.ts`/Widget#
  //                   ^^^^^^ reference scippy npm workspace . `widget.ts`/Widget#
  export const m = mk();
//             ^ definition scippy npm workspace . `use.ts`/m.
//               kind Variable
//               display_name m
//                 ^^ reference scippy npm workspace . `make.ts`/make().
  //               ^^ reference scippy npm workspace . `make.ts`/make().
  export const c = cfg;
//             ^ definition scippy npm workspace . `use.ts`/c.
//               kind Variable
//               display_name c
//                 ^^^ reference scippy npm workspace . `config.ts`/config.
  //               ^^^ reference scippy npm workspace . `config.ts`/config.
  export const l = lit;
//             ^ definition scippy npm workspace . `use.ts`/l.
//               kind Variable
//               display_name l
//                 ^^^ reference scippy npm workspace . `literal.ts`/default.
  //               ^^^ reference scippy npm workspace . `literal.ts`/default.
  
