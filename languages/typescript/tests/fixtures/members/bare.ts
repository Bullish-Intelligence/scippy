export const area = 1;
//           ^^^^ definition scippy npm workspace . `bare.ts`/area.
export class Box {
//           ^^^ definition scippy npm workspace . `bare.ts`/Box#
  area(): number {
//^^^^ definition scippy npm workspace . `bare.ts`/Box#area().
    return area;
//         ^^^^ reference scippy npm workspace . `bare.ts`/area.
  }
}
