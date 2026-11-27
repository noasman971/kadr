<!--
  One imported file in the library: thumbnail, kind, length, whether it is used,
  a + button that adds it at the end of the timeline, and the state of its proxy
  (heavy videos, decision record 0055) with the action that fits.
-->
<script lang="ts">
  import type { MediaView } from "../../api/types";
  import { t } from "../../i18n/i18n.svelte";
  import Thumbnail from "../../media/Thumbnail.svelte";
  import { proxyBadgeOf } from "../../media/proxyBadge";
  import { proxies } from "../../state/proxies.svelte";
  import { TICKS_PER_SECOND, formatShortDuration } from "../../time/ticks";
  import Icon from "../../ui/Icon.svelte";
  import type { IconName } from "../../ui/icons";

  interface Props {
    media: MediaView;
    onAddAtEnd: (media: MediaView) => void;
    onPointerDown?: (event: PointerEvent, media: MediaView) => void;
  }

  let { media, onAddAtEnd, onPointerDown }: Props = $props();

  const KIND_ICONS: Record<MediaView["kind"], IconName> = { video: "film", audio: "music", image: "image" };
  /** Thumbnails show one second in, or the middle of shorter videos. */
  const thumbnailTimeTicks = $derived(Math.min(TICKS_PER_SECOND, (media.durationTicks ?? 0) / 2));
  const proxyBadge = $derived(media.isMissing ? null : proxyBadgeOf(proxies.stateOf(media.filePath), media));

  function runProxyAction(): void {
    if (proxyBadge?.action === "stop") {
      void proxies.stop(media.id);
    } else if (proxyBadge?.action === "makeAgain") {
      void proxies.makeAgain(media.id);
    }
  }
</script>

<div
  class="tile"
  class:missing={media.isMissing}
  role="listitem"
  title={media.isMissing ? t("library.missing", { name: media.name }) : media.filePath}
  onpointerdown={(event) => onPointerDown?.(event, media)}
>
  <div class="picture">
    {#if media.hasPicture && !media.isMissing}
      <Thumbnail
        mediaId={media.id}
        filePath={media.filePath}
        sourceTimeTicks={thumbnailTimeTicks}
        maxWidth={320}
        maxHeight={180}
      />
    {:else}
      <div class="placeholder {media.kind}" aria-hidden="true">
        <Icon name={media.isMissing ? "alert" : KIND_ICONS[media.kind]} size={24} />
      </div>
    {/if}
    <span class="kind" title={t(`kind.${media.kind}`)}><Icon name={KIND_ICONS[media.kind]} size={12} /></span>
    {#if proxyBadge}
      <span class="proxy {proxyBadge.tone}" title={proxyBadge.hint}>
        <span class="proxy-label tabular">{proxyBadge.label}</span>
        {#if proxyBadge.action}
          <button
            type="button"
            class="proxy-action"
            title={t(proxyBadge.action === "stop" ? "library.proxy.stop" : "library.proxy.makeAgain")}
            aria-label="{t(proxyBadge.action === 'stop' ? 'library.proxy.stop' : 'library.proxy.makeAgain')}: {media.name}"
            onpointerdown={(event) => event.stopPropagation()}
            onclick={runProxyAction}
          >
            <Icon name={proxyBadge.action === "stop" ? "close" : "rotate"} size={10} />
          </button>
        {/if}
      </span>
      {#if proxyBadge.fraction !== null}
        <span class="proxy-progress" aria-hidden="true" style:width="{proxyBadge.fraction * 100}%"></span>
      {/if}
    {/if}
    {#if media.useCount > 0}
      <span class="used" title={t("library.inTimeline")}><Icon name="check" size={11} /></span>
    {/if}
    {#if media.durationTicks !== null}
      <span class="duration tabular">{formatShortDuration(media.durationTicks)}</span>
    {/if}
    {#if !media.isMissing}
      <button
        type="button"
        class="add"
        title={t("library.addAtEnd")}
        aria-label="{t('library.addAtEnd')}: {media.name}"
        onpointerdown={(event) => event.stopPropagation()}
        onclick={() => onAddAtEnd(media)}
      >
        <Icon name="plus" size={14} />
      </button>
    {/if}
  </div>
  <span class="name">{media.isMissing ? t("library.missing", { name: media.name }) : media.name}</span>
</div>

<style>
  .tile {
    position: relative;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    cursor: grab;
    touch-action: none;
  }
  .picture {
    position: relative;
    aspect-ratio: 16 / 9;
    border-radius: var(--radius-md);
    overflow: hidden;
    background: var(--color-bg-inset);
    border: 1px solid var(--color-border);
  }
  .tile:hover .picture {
    border-color: var(--color-border-strong);
  }
  .placeholder {
    width: 100%;
    height: 100%;
    display: grid;
    place-items: center;
    color: rgba(255, 255, 255, 0.85);
  }
  .placeholder.audio {
    background: linear-gradient(135deg, var(--color-clip-audio), #0f3d33);
  }
  .placeholder.video,
  .placeholder.image {
    background: var(--color-bg-active);
    color: var(--color-text-muted);
  }
  .missing .placeholder {
    background: var(--color-warning-soft);
    color: var(--color-warning);
  }
  .missing .name {
    color: var(--color-warning);
  }
  .kind {
    position: absolute;
    top: 4px;
    left: 4px;
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: var(--radius-sm);
    background: rgba(0, 0, 0, 0.55);
    color: #ffffff;
  }
  .proxy {
    position: absolute;
    top: 4px;
    left: 28px;
    max-width: calc(100% - 60px);
    height: 20px;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 0 2px 0 6px;
    border-radius: var(--radius-sm);
    background: rgba(0, 0, 0, 0.6);
    color: #ffffff;
    font-size: var(--font-size-xs);
    white-space: nowrap;
  }
  .proxy:not(:has(.proxy-action)) {
    padding-right: 6px;
  }
  .proxy.ready {
    color: #3ccf91;
  }
  .proxy.warning {
    color: var(--color-warning);
  }
  .proxy-label {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .proxy-action {
    width: 16px;
    height: 16px;
    flex: none;
    display: grid;
    place-items: center;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: inherit;
    cursor: pointer;
  }
  .proxy-action:hover,
  .proxy-action:focus-visible {
    background: rgba(255, 255, 255, 0.2);
  }
  .proxy-progress {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 3px;
    background: var(--color-accent);
    transition: width var(--duration-fast);
  }
  .used {
    position: absolute;
    bottom: 4px;
    left: 4px;
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: rgba(0, 0, 0, 0.6);
    color: #3ccf91;
  }
  .duration {
    position: absolute;
    bottom: 4px;
    right: 4px;
    padding: 1px 5px;
    border-radius: var(--radius-sm);
    background: rgba(0, 0, 0, 0.6);
    color: #ffffff;
    font-size: var(--font-size-xs);
  }
  .add {
    position: absolute;
    top: 4px;
    right: 4px;
    width: 24px;
    height: 24px;
    border: 0;
    border-radius: var(--radius-sm);
    background: var(--color-accent);
    color: var(--color-on-accent);
    display: grid;
    place-items: center;
    cursor: pointer;
    opacity: 0;
    transition: opacity var(--duration-fast);
  }
  .tile:hover .add,
  .add:focus-visible {
    opacity: 1;
  }
  .name {
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
