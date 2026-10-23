<script lang="ts">
  import { tick } from 'svelte';
  import { dialogs } from '../dialogs.svelte';
  import { t } from '../i18n';
  import Dialog from './Dialog.svelte';

  let value = $state('');
  let input = $state<HTMLInputElement>();
  const current = $derived(dialogs.current);

  $effect(() => {
    if (current?.kind === 'prompt') {
      value = current.value;
      void tick().then(() => {
        input?.focus();
        input?.select();
      });
    }
  });
</script>

{#if current}
  <Dialog title={current.title} width={440} onclose={() => dialogs.finish(null)}>
    {#if current.message}<p class="message secondary">{current.message}</p>{/if}
    {#if current.kind === 'prompt'}
      <input
        class="input"
        bind:this={input}
        bind:value
        placeholder={current.placeholder}
        onkeydown={(event) => {
          if (event.key === 'Enter' && value.trim()) dialogs.finish(value.trim());
        }}
      />
    {/if}
    {#snippet footer()}
      {#if current.kind === 'prompt'}
        <button class="button accent" disabled={!value.trim()} onclick={() => dialogs.finish(value.trim())}>
          {current.confirm}
        </button>
      {:else}
        <button class="button accent" class:danger-accent={current.danger} onclick={() => dialogs.finish(true)}>
          {current.confirm}
        </button>
      {/if}
      <button class="button" onclick={() => dialogs.finish(null)}>{t('Abbrechen')}</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .message {
    margin: 0;
    white-space: pre-line;
  }
  .danger-accent {
    background: var(--red) !important;
    color: #fff !important;
  }
</style>
