import { createApp } from 'vue';
import { createPinia } from 'pinia';
import ToastWindow from './components/notifications/ToastWindow.vue';
import ImportantAlerts from './components/notifications/ImportantAlerts.vue';
import { getCurrentWindow } from '@tauri-apps/api/window';
import './assets/styles/variables.css';
createApp(getCurrentWindow().label === 'important-alerts' ? ImportantAlerts : ToastWindow).use(createPinia()).mount('#app');
