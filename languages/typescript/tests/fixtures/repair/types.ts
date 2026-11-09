export interface Reader<out T> {
//               ^^^^^^ definition scippy npm workspace . `types.ts`/Reader#
  read(): T;
//^^^^ definition scippy npm workspace . `types.ts`/Reader#read().
}
export interface Caller {
//               ^^^^^^ definition scippy npm workspace . `types.ts`/Caller#
  <A>(a: A): A
  <B>(a: B, b: B): B
}
export const call: Caller = (a: any) => a;
//           ^^^^ definition scippy npm workspace . `types.ts`/call.
