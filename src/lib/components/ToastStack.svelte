<script lang="ts">
  import type { Toast } from "$lib/app-model";

  let {
    toasts,
    onDismiss,
  }: {
    toasts: Toast[];
    onDismiss: (id: number) => void;
  } = $props();
</script>

{#if toasts.length}
  <div class="toast-stack" aria-live="polite">
    {#each toasts as t (t.id)}
      <button class="toast toast-{t.kind}" onclick={() => onDismiss(t.id)} title="Dismiss">
        {t.message}
      </button>
    {/each}
  </div>
{/if}

<style>
  .toast-stack {
    position: fixed;
    right: 1rem;
    bottom: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    z-index: 1000;
    pointer-events: none;
  }
  .toast {
    pointer-events: auto;
    max-width: 360px;
    padding: 0.6rem 0.85rem;
    background: var(--panel);
    color: var(--fg);
    border: 1px solid var(--border-2);
    border-left: 3px solid #c08a3e;
    border-radius: 2px;
    font: inherit;
    font-size: 0.75rem;
    text-align: left;
    cursor: pointer;
    box-shadow: 0 4px 16px rgba(0,0,0,0.35);
    animation: toast-in 160ms ease-out;
  }
  .toast-error { border-left-color: #c2553a; }
  .toast:hover { background: var(--panel-2); }
  @keyframes toast-in {
    from { opacity: 0; transform: translateY(6px); }
    to { opacity: 1; transform: translateY(0); }
  }
</style>
