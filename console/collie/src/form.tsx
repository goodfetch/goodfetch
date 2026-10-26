// The Console's one form element (ADR 0073): a `<form>` whose Enter submits
// nothing while an input method composes text in one of its fields, so a
// person typing Japanese, Chinese or Korean commits a word with Enter without
// sending a half-typed value or running a search. An input method's Enter
// comes while the composition is open (`isComposing`), or as the key code
// 229 that Safari sends just after the composition ends. Every form and
// search field of the Console renders this one, never a bare `<form>`.
import type { ComponentProps, KeyboardEvent, ReactNode } from "react";

/** Keeps the Enter that commits an input method's composition from submitting `event`'s form. */
function holdComposingEnter(event: KeyboardEvent<HTMLFormElement>): void {
  // oxlint-disable-next-line typescript/no-deprecated -- `keyCode` 229 is the only sign of Safari's Enter that ends a composition, which `isComposing` misses.
  if (event.key === "Enter" && (event.nativeEvent.isComposing || event.keyCode === 229)) {
    event.preventDefault();
  }
}

/** A `<form>` whose Enter submits nothing during an input method's composition. */
export function Form({ onKeyDown, ...props }: ComponentProps<"form">): ReactNode {
  return (
    // oxlint-disable-next-line jsx-a11y/no-noninteractive-element-interactions -- the form only holds back its fields' composing Enter; it takes no input of its own.
    <form
      {...props}
      onKeyDown={(event) => {
        holdComposingEnter(event);
        onKeyDown?.(event);
      }}
    />
  );
}
