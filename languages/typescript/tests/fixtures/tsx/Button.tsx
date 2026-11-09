import { shout } from "./text";
//       ^^^^^ reference scippy npm workspace . `text.ts`/shout().
const button = 1;
//    ^^^^^^ definition scippy npm workspace . `Button.tsx`/button.
export function Button(props: { label: string }) {
//              ^^^^^^ definition scippy npm workspace . `Button.tsx`/Button().
  return <button>{shout(props.label)}</button>;
//                ^^^^^ reference scippy npm workspace . `text.ts`/shout().
}
