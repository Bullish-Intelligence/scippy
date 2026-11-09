export enum Color {
//          ^^^^^ definition scippy npm workspace . `enums.ts`/Color#
  Red,
//^^^ definition scippy npm workspace . `enums.ts`/Color#Red.
  Green = 2,
//^^^^^ definition scippy npm workspace . `enums.ts`/Color#Green.
  Blue = Red,
//^^^^ definition scippy npm workspace . `enums.ts`/Color#Blue.
}
export const pick = Color.Green;
//                  ^^^^^ reference scippy npm workspace . `enums.ts`/Color#
//                        ^^^^^ reference scippy npm workspace . `enums.ts`/Color#Green.
export namespace Geo {
//               ^^^ definition scippy npm workspace . `enums.ts`/Geo/
  export const PI = 3;
//             ^^ definition scippy npm workspace . `enums.ts`/Geo/PI.
  export function area(r: number): number {
//                ^^^^ definition scippy npm workspace . `enums.ts`/Geo/area().
    return PI * r;
//         ^^ reference scippy npm workspace . `enums.ts`/Geo/PI.
  }
}
export const a = Geo.area(Geo.PI);
//               ^^^ reference scippy npm workspace . `enums.ts`/Geo/
//                   ^^^^ reference scippy npm workspace . `enums.ts`/Geo/area().
//                        ^^^ reference scippy npm workspace . `enums.ts`/Geo/
//                            ^^ reference scippy npm workspace . `enums.ts`/Geo/PI.
