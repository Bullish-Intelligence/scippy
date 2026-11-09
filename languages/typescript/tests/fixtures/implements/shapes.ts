export interface Shape {
//               ^^^^^ definition scippy npm workspace . `shapes.ts`/Shape#
  area(): number;
//^^^^ definition scippy npm workspace . `shapes.ts`/Shape#area().
}
export class Base {
//           ^^^^ definition scippy npm workspace . `shapes.ts`/Base#
  describe(): string {
//^^^^^^^^ definition scippy npm workspace . `shapes.ts`/Base#describe().
    return "base";
  }
}
export class Square extends Base implements Shape {
//           ^^^^^^ definition scippy npm workspace . `shapes.ts`/Square#
//                          ^^^^ reference scippy npm workspace . `shapes.ts`/Base#
//                                          ^^^^^ reference scippy npm workspace . `shapes.ts`/Shape#
  area(): number {
//^^^^ definition scippy npm workspace . `shapes.ts`/Square#area().
    return 1;
  }
  describe(): string {
//^^^^^^^^ definition scippy npm workspace . `shapes.ts`/Square#describe().
    return "square";
  }
}
