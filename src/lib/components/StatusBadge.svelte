<script lang="ts">
  import { percent } from '../format';
  import { t } from '../i18n';
  import type { IconName } from '../icons';
  import type { TranscriptionStatus } from '../types';
  import Icon from './Icon.svelte';

  let { status, progress = 0, compact = false }: { status: TranscriptionStatus; progress?: number; compact?: boolean } =
    $props();

  const looks: Record<TranscriptionStatus, { label: string; icon: IconName; color: string }> = {
    notTranscribed: { label: t('Nicht transkribiert'), icon: 'documentText', color: 'var(--text-2)' },
    queued: { label: t('In Warteschlange'), icon: 'clock', color: 'var(--orange)' },
    running: { label: t('Läuft'), icon: 'hourglass', color: 'var(--accent-text)' },
    done: { label: t('Fertig'), icon: 'checkmarkCircle', color: 'var(--green)' },
    failed: { label: t('Fehler'), icon: 'warning', color: 'var(--red)' },
  };
  const look = $derived(looks[status]);
</script>

<span class="badge caption" style:color={look.color} title={look.label}>
  {#if status === 'running'}
    <span class="pulse"><Icon name={look.icon} size={14} /></span>
  {:else}
    <Icon name={look.icon} size={14} />
  {/if}
  {#if status === 'running' && progress > 0}
    <span class="tabular">{percent(progress)}</span>
  {:else if !compact}
    <span>{look.label}</span>
  {/if}
</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border-radius: 10px;
    font-weight: 600;
    white-space: nowrap;
    background: color-mix(in srgb, currentColor 13%, transparent);
  }
  .pulse {
    display: inline-flex;
    animation: pulse 1.2s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
</style>
