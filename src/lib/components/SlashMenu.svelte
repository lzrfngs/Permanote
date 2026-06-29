<script lang="ts">
  import type { SlashItem } from "$lib/slash-command";

  let {
    items,
    activeIndex,
    left,
    top,
    onPick,
    onHover,
  }: {
    items: SlashItem[];
    activeIndex: number;
    left: number;
    top: number;
    onPick: (index: number) => void;
    onHover: (index: number) => void;
  } = $props();
</script>

{#if items.length > 0}
  <div
    class="slash-menu"
    style="left: {left}px; top: {top}px;"
    role="listbox"
  >
    {#each items as item, i (item.title)}
      <button
        class="slash-item"
        class:active={i === activeIndex}
        onmousedown={(e) => { e.preventDefault(); onPick(i); }}
        onmouseenter={() => onHover(i)}
        type="button"
      >
        <span class="slash-title">{item.title}</span>
        {#if item.hint}<span class="slash-hint">{item.hint}</span>{/if}
      </button>
    {/each}
  </div>
{/if}

<style>
  .slash-menu {
    position: fixed;
    z-index: 1000;
    min-width: 220px;
    max-height: 280px;
    overflow-y: auto;
    background: var(--panel-2);
    border: 1px solid var(--border-2);
    border-radius: 4px;
    padding: 4px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-size: 0.72rem;
  }
  .slash-item {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    width: 100%;
    background: transparent;
    border: 0;
    color: var(--fg);
    padding: 6px 10px;
    text-align: left;
    cursor: pointer;
    border-radius: 2px;
  }
  .slash-item.active {
    background: var(--accent-bg);
  }
  .slash-title {
    color: var(--fg);
  }
  .slash-hint {
    color: var(--fg-faint);
    font-size: 0.62rem;
    margin-left: 12px;
  }
</style>
