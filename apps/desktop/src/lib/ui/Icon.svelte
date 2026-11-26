<!--
  An icon from `icons.ts`. Decorative by default (hidden from screen readers):
  the button or text around it carries the meaning.
-->
<script lang="ts">
  import { ICON_SHAPES, type IconName } from "./icons";

  interface Props {
    name: IconName;
    size?: number;
    filled?: boolean;
  }

  let { name, size = 16, filled = false }: Props = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill={filled ? "currentColor" : "none"}
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"
>
  <!-- The shapes are constants of this app, never user content. -->
  {@html ICON_SHAPES[name]}
</svg>

<style>
  svg {
    flex: none;
    display: block;
  }
</style>
