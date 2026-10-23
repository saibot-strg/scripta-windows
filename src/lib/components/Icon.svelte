<script lang="ts">
  import { icons, type IconName } from '../icons';

  let { name, size = 16, class: className = '' }: { name: IconName; size?: number; class?: string } = $props();
</script>

<span class="icon {className}" style:width="{size}px" style:height="{size}px" aria-hidden="true">
  {@html icons[name]}
</span>

<style>
  .icon {
    display: inline-flex;
    flex: none;
  }
  .icon :global(svg) {
    width: 100%;
    height: 100%;
    fill: currentColor;
  }
</style>
