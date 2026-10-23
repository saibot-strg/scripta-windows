import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';

// Browser preview with sample data (development only).
const preview = import.meta.env.DEV && !('__TAURI_INTERNALS__' in window) ? (await import('./mock')).preview : null;

const app = mount(App, { target: document.getElementById('app')! });

if (preview) {
  const { app: store } = await import('./lib/store.svelte');
  const ready = () =>
    new Promise<void>((resolve) => {
      const check = () => (store.ready ? resolve() : setTimeout(check, 20));
      check();
    });
  await ready();
  if (preview.style === 'text') store.setTranscriptStyle('text');
  if (preview.select !== null) store.select(store.visible[Number(preview.select)]?.id ?? null);
  if (preview.settings) store.openSettings(preview.settings);
  if (preview.assign) setTimeout(() => (store.assignmentOpen = true), 200);
}

export default app;
