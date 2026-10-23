<script lang="ts">
  import type { IconName } from '../icons';
  import type { RecordingSource } from '../types';
  import Icon from './Icon.svelte';

  let { source, size = 32 }: { source: RecordingSource; size?: number } = $props();

  const looks: Record<RecordingSource, { icon: IconName; color: string }> = {
    microphone: { icon: 'micFilled', color: '#e0443a' },
    meeting: { icon: 'people', color: '#2e9e4f' },
    importedFile: { icon: 'arrowDownload', color: '#2f7fe0' },
  };
</script>

<span
  class="source"
  style:width="{size}px"
  style:height="{size}px"
  style:border-radius="{size * 0.26}px"
  style:background="linear-gradient(180deg, color-mix(in srgb, {looks[source].color} 85%, white), {looks[source].color})"
  aria-hidden="true"
>
  <Icon name={looks[source].icon} size={Math.round(size * 0.55)} />
</span>

<style>
  .source {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    color: white;
  }
</style>
