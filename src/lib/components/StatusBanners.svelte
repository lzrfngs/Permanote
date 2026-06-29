<script lang="ts">
  let {
    hasExternalConflict,
    hasPendingUpdate,
    updateVersion,
    updateBusy,
    onKeepMine,
    onUseTheirs,
    onInstallUpdate,
    onDismissUpdate,
  }: {
    hasExternalConflict: boolean;
    hasPendingUpdate: boolean;
    updateVersion: string;
    updateBusy: boolean;
    onKeepMine: () => void;
    onUseTheirs: () => void;
    onInstallUpdate: () => void | Promise<void>;
    onDismissUpdate: () => void;
  } = $props();
</script>

{#if hasExternalConflict}
  <div class="conflict-banner" role="alert">
    <span class="conflict-text">
      This day was changed on another device.
    </span>
    <div class="conflict-actions">
      <button onclick={onKeepMine}>Keep mine</button>
      <button onclick={onUseTheirs}>Use theirs</button>
    </div>
  </div>
{/if}

{#if hasPendingUpdate}
  <div class="update-banner" role="status">
    <span class="update-text">
      Permanote {updateVersion} is available.
    </span>
    <div class="conflict-actions">
      <button onclick={onInstallUpdate} disabled={updateBusy}>
        {updateBusy ? "Installing..." : "Update & restart"}
      </button>
      <button onclick={onDismissUpdate} disabled={updateBusy}>Later</button>
    </div>
  </div>
{/if}

<style>
  .conflict-banner,
  .update-banner {
    position: fixed;
    left: 50%;
    bottom: 2rem;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 8px 14px;
    border-radius: 3px;
    color: var(--fg);
    font-size: 0.72rem;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
    z-index: 200;
  }
  .conflict-banner {
    background: #1a140a;
    border: 1px solid #c08a3e;
  }
  .conflict-text {
    color: #d8b078;
  }
  .update-banner {
    background: #0c1622;
    border: 1px solid #4a78c0;
  }
  .update-text {
    color: #9cc0f0;
  }
  .update-banner button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .conflict-actions {
    display: flex;
    gap: 6px;
  }
  .conflict-actions button {
    background: transparent;
    border: 1px solid var(--border-2);
    color: var(--fg);
    font: inherit;
    font-size: 0.7rem;
    padding: 3px 10px;
    cursor: pointer;
    border-radius: 2px;
  }
  .conflict-actions button:hover {
    background: var(--accent-bg);
    border-color: var(--border-3);
  }
</style>
