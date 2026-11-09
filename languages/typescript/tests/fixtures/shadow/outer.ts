export function process(x: number): number {
//              ^^^^^^^ definition scippy npm workspace . `outer.ts`/process().
  return x + 1;
}
export const first = process(2);
//                   ^^^^^^^ reference scippy npm workspace . `outer.ts`/process().
export function shadowed(): number {
  const process = 5;
//      ^^^^^^^ definition local 1
  return process;
//       ^^^^^^^ reference local 1
}
