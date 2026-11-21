import type { MouseEvent } from "react";

/** Shows the full text as a tooltip only when it is cut off. */
export function clippedTitle(event: MouseEvent<HTMLElement>) {
  const text = event.currentTarget;
  text.title =
    text.scrollWidth > text.clientWidth ? (text.textContent ?? "") : "";
}
