import { Counter } from "./counter";
//       ^^^^^^^ reference scippy npm workspace . `counter.ts`/Counter#
const ctr = new Counter(1);
//    ^^^ definition scippy npm workspace . `use.ts`/ctr.
//              ^^^^^^^ reference scippy npm workspace . `counter.ts`/Counter#
ctr.inc();
// <- reference scippy npm workspace . `use.ts`/ctr.
//  ^^^ reference scippy npm workspace . `counter.ts`/Counter#inc().
export const n = ctr.count + ctr.value + Counter.created;
//                   ^^^^^ reference scippy npm workspace . `counter.ts`/Counter#count.
//                               ^^^^^ reference scippy npm workspace . `counter.ts`/Counter#value().
//                                       ^^^^^^^ reference scippy npm workspace . `counter.ts`/Counter#
//                                               ^^^^^^^ reference scippy npm workspace . `counter.ts`/Counter#created.
