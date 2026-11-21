/**
 * Plays the pin's press animation in place. Run on the icon itself, as
 * React rewrites the button's class when the pin state changes.
 */
export function popPin(button: HTMLElement) {
  const icon = button.querySelector("svg");
  if (
    !icon ||
    matchMedia("(prefers-reduced-motion: reduce)").matches ||
    button.closest(".reduce-motion")
  )
    return;
  icon.animate(
    [
      { scale: "1", rotate: "0deg" },
      { scale: "0.72", rotate: "-18deg", offset: 0.35 },
      { scale: "1.14", rotate: "6deg", offset: 0.7 },
      { scale: "1", rotate: "0deg" },
    ],
    { duration: 420, easing: "cubic-bezier(0.3, 0, 0.2, 1)" },
  );
}
