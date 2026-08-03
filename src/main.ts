import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import { applyInitialTheme } from './lib/pwa/theme';
import { registerServiceWorker } from './lib/pwa/register';

// Apply the persisted/OS theme before mounting so there's no theme flash.
applyInitialTheme();

const target = document.getElementById('app');
if (!target) {
  throw new Error('Mount target #app not found');
}

const app = mount(App, { target });

// Register the service worker after mount so it never blocks first paint.
registerServiceWorker();

export default app;
