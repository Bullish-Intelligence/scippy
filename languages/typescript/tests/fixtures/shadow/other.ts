export function process(x: string): string {
//              ^^^^^^^ definition scippy npm workspace . `other.ts`/process().
  return x;
}
export const second = process('a');
//                    ^^^^^^^ reference scippy npm workspace . `other.ts`/process().
