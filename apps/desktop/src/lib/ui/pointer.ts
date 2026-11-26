// Pointer positions in the interface's own pixels.
//
// The interface size setting scales the page with CSS `zoom` (base.css). Pointer
// positions (`clientX`) and `getBoundingClientRect()` are then in zoomed pixels,
// while element sizes, `scrollLeft` and the positions the timeline computes are
// not. Every pointer position the timeline turns into a time goes through here.

/** A pointer position as `left` / `top` of a `position: fixed` element inside the zoomed page. */
export function unzoomedClientPosition(clientX: number, clientY: number): { x: number; y: number } {
  const zoom = document.body.currentCSSZoom || 1;
  return { x: clientX / zoom, y: clientY / zoom };
}

/** How far `clientX` is from the left edge of `element`, in the element's own CSS pixels. */
export function pointerOffsetX(clientX: number, element: Element): number {
  const zoom = element.currentCSSZoom || 1;
  return (clientX - element.getBoundingClientRect().left) / zoom;
}
