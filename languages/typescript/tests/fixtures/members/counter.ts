export class Counter {
//           ^^^^^^^ definition scippy npm workspace . `counter.ts`/Counter#
  static created = 0;
//       ^^^^^^^ definition scippy npm workspace . `counter.ts`/Counter#created.
  count = 0;
//^^^^^ definition scippy npm workspace . `counter.ts`/Counter#count.
  constructor(private step: number) {
//^^^^^^^^^^^ definition scippy npm workspace . `counter.ts`/Counter#constructor().
//                    ^^^^ definition scippy npm workspace . `counter.ts`/Counter#step.
    Counter.created += 1;
//  ^^^^^^^ reference scippy npm workspace . `counter.ts`/Counter#
//          ^^^^^^^ reference scippy npm workspace . `counter.ts`/Counter#created.
    this.count = step;
//       ^^^^^ reference scippy npm workspace . `counter.ts`/Counter#count.
//               ^^^^ reference scippy npm workspace . `counter.ts`/Counter#step.
    this.step += this.count;
//       ^^^^ reference scippy npm workspace . `counter.ts`/Counter#step.
  }
  inc(): void {
//^^^ definition scippy npm workspace . `counter.ts`/Counter#inc().
    this.count += 1;
//       ^^^^^ reference scippy npm workspace . `counter.ts`/Counter#count.
  }
  get value(): number {
//    ^^^^^ definition scippy npm workspace . `counter.ts`/Counter#value().
    return this.count;
//              ^^^^^ reference scippy npm workspace . `counter.ts`/Counter#count.
  }
}
